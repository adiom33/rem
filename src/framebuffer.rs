use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::ptr;

use crate::font;
use crate::sys;

// ---- MXCFB ioctl structures for e-ink refresh ----

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MxcfbRect {
    pub top: u32,
    pub left: u32,
    pub width: u32,
    pub height: u32,
}

/// V2 update struct (72 bytes) — used by newer EPDC kernels and some RM2 firmware.
/// Also used as the payload in rm2fb message queue messages.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MxcfbUpdateData {
    pub update_region: MxcfbRect,
    pub waveform_mode: u32,
    pub update_mode: u32,
    pub update_marker: u32,
    pub temp: i32,
    pub flags: u32,
    pub dither_mode: i32,
    pub quant_bit: i32,
    pub alt_buffer_data: [u8; 28],
}

/// V1 update struct (36 bytes) — used by older EPDC kernels.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MxcfbUpdateDataV1 {
    pub update_region: MxcfbRect,
    pub waveform_mode: u32,
    pub update_mode: u32,
    pub update_marker: u32,
    pub temp: i32,
    pub flags: u32,
}

/// Sys V message queue envelope for rm2fb.
/// The mtype field is required by msgsnd (must be > 0).
/// The payload is the same mxcfb_update_data struct that the ioctl would use.
#[repr(C)]
struct SwtfbUpdateMsg {
    mtype: sys::c_long,
    update: MxcfbUpdateData,
}

// rm2fb shared memory: fixed 1404×1872 @ 16bpp RGB565
const RM2FB_WIDTH: usize = 1404;
const RM2FB_HEIGHT: usize = 1872;
const RM2FB_BPP: usize = 16;
const RM2FB_SHM_SIZE: usize = RM2FB_WIDTH * RM2FB_HEIGHT * (RM2FB_BPP / 8);
const RM2FB_SHM_PATH: &str = "/dev/shm/swtfb.01";

// Waveform modes
pub const WAVEFORM_MODE_INIT: u32 = 0;
pub const WAVEFORM_MODE_DU: u32 = 1;
pub const WAVEFORM_MODE_GC16: u32 = 2;
#[allow(dead_code)]
pub const WAVEFORM_MODE_AUTO: u32 = 257;

// Update modes
pub const UPDATE_MODE_PARTIAL: u32 = 0;
pub const UPDATE_MODE_FULL: u32 = 1;

// Temperature
pub const TEMP_USE_AMBIENT: i32 = 0x1000;

// ---- Framebuffer variable screen info (from linux/fb.h) ----

#[repr(C)]
#[derive(Debug)]
struct FbVarScreeninfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    _rest: [u8; 128],
}

impl Default for FbVarScreeninfo {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Debug)]
struct FbFixScreeninfo {
    id: [u8; 16],
    smem_start: usize,
    smem_len: u32,
    fb_type: u32,
    _rest: [u8; 200],
}

impl Default for FbFixScreeninfo {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

/// Display backend: how we push pixels to the e-ink panel.
#[derive(Clone, Copy, PartialEq)]
enum DisplayBackend {
    /// Direct MXCFB_SEND_UPDATE V2 ioctl on /dev/fb0 (RM1, or RM2 with working EPDC)
    NativeV2,
    /// Direct MXCFB_SEND_UPDATE V1 ioctl on /dev/fb0
    NativeV1,
    /// rm2fb: pixels via shared memory, updates via Sys V message queue.
    /// No LD_PRELOAD needed — we speak the protocol natively.
    Rm2fb { msqid: sys::c_int },
    /// FBIOPAN_DISPLAY fallback: write to fb0 + pan to push frames through the
    /// mxsfb LCD controller. On RM2 this pushes raw pixels without waveform
    /// processing — the e-ink panel may show very faint/degraded updates but
    /// it's better than nothing when rm2fb is unavailable.
    FbPan,
    /// Nothing works.
    None,
}

/// Read the reMarkable firmware version from the filesystem.
/// Returns something like "3.25.1.1" or None if not readable.
fn read_firmware_version() -> Option<String> {
    // Primary: /usr/share/remarkable/update.conf contains REMARKABLE_RELEASE_VERSION=x.y.z
    if let Ok(contents) = std::fs::read_to_string("/usr/share/remarkable/update.conf") {
        for line in contents.lines() {
            if let Some(ver) = line.strip_prefix("REMARKABLE_RELEASE_VERSION=") {
                return Some(ver.trim().to_string());
            }
        }
    }
    // Fallback: /etc/version
    if let Ok(ver) = std::fs::read_to_string("/etc/version") {
        let trimmed = ver.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Detect device model from /sys/devices. Returns "reMarkable 1", "reMarkable 2", or "unknown".
fn detect_device_model() -> &'static str {
    if let Ok(dt) = std::fs::read_to_string("/sys/firmware/devicetree/base/model") {
        let dt = dt.trim_end_matches('\0');
        if dt.contains("reMarkable 2") || dt.contains("zero-sugar") {
            return "reMarkable 2";
        }
        if dt.contains("reMarkable") {
            return "reMarkable 1";
        }
    }
    // Fallback: check for RM2's SWTCON marker
    if std::path::Path::new("/sys/class/graphics/fb0/device/driver").exists() {
        if let Ok(link) = std::fs::read_link("/sys/class/graphics/fb0/device/driver") {
            let driver_name = link
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if driver_name.contains("mxsfb") || driver_name.contains("lcdif") {
                return "reMarkable 2";
            }
            if driver_name.contains("mxc_epdc") {
                return "reMarkable 1";
            }
        }
    }
    "unknown"
}

pub struct Framebuffer {
    file: File,
    pub width: usize,
    pub height: usize,
    pub bits_per_pixel: usize,
    line_length: usize,
    /// Active pixel memory — either /dev/fb0 mmap or rm2fb shared memory.
    mem: *mut u8,
    mem_len: usize,
    /// /dev/fb0 mmap (always kept for cleanup)
    fb0_mem: *mut u8,
    fb0_mem_len: usize,
    /// rm2fb shared memory mmap (if using rm2fb backend)
    rm2fb_mem: *mut u8,
    rm2fb_mem_len: usize,
    update_marker: u32,
    partial_refresh_count: u32,
    backend: DisplayBackend,
}

unsafe impl Send for Framebuffer {}

impl Framebuffer {
    pub fn open(path: &str) -> Result<Self, String> {
        // Log device info upfront for diagnostics
        let device_model = detect_device_model();
        let firmware_ver = read_firmware_version();
        eprintln!("Device: {}", device_model);
        if let Some(ref ver) = firmware_ver {
            eprintln!("Firmware: {}", ver);
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("Cannot open {}: {}", path, e))?;

        let fd = file.as_raw_fd();

        // Get variable screen info
        let mut vinfo = FbVarScreeninfo::default();
        let ret = unsafe { sys::ioctl(fd, sys::FBIOGET_VSCREENINFO, &mut vinfo) };
        if ret < 0 {
            return Err(format!("FBIOGET_VSCREENINFO failed: {} (errno {})", sys::errno_str(), sys::errno()));
        }

        // Get fixed screen info
        let mut finfo = FbFixScreeninfo::default();
        let ret = unsafe { sys::ioctl(fd, sys::FBIOGET_FSCREENINFO, &mut finfo) };
        if ret < 0 {
            return Err(format!("FBIOGET_FSCREENINFO failed: {} (errno {})", sys::errno_str(), sys::errno()));
        }

        let width = vinfo.xres as usize;
        let height = vinfo.yres as usize;
        let bpp = vinfo.bits_per_pixel as usize;
        let bytes_per_pixel = if bpp > 0 { bpp / 8 } else { 2 };
        let line_length = width * bytes_per_pixel;
        let fb0_mem_len = if line_length > 0 && height > 0 {
            line_length * height
        } else {
            // Fallback: use finfo.smem_len or a reasonable default
            if finfo.smem_len > 0 { finfo.smem_len as usize } else { RM2FB_SHM_SIZE }
        };

        let fb_id_end = finfo.id.iter().position(|&b| b == 0).unwrap_or(finfo.id.len());
        let fb_id = String::from_utf8_lossy(&finfo.id[..fb_id_end]);

        eprintln!(
            "Framebuffer: {}x{} @ {}bpp ({}KB) driver=\"{}\"",
            width, height, bpp, fb0_mem_len / 1024, fb_id
        );

        let is_rm2_driver = {
            let id_lower = fb_id.to_lowercase();
            id_lower.contains("mxs") || id_lower.contains("lcdif") || id_lower.contains("mxsfb")
        };
        if is_rm2_driver {
            eprintln!("  (RM2 software display controller — native MXCFB ioctls will not work)");
        }

        // mmap /dev/fb0 (we always do this even if we switch to rm2fb later)
        let fb0_mem = unsafe {
            sys::mmap(
                ptr::null_mut(),
                fb0_mem_len,
                sys::PROT_READ | sys::PROT_WRITE,
                sys::MAP_SHARED,
                fd,
                0,
            )
        };
        if fb0_mem == sys::MAP_FAILED {
            return Err(format!("mmap /dev/fb0 failed: {} (errno {})", sys::errno_str(), sys::errno()));
        }

        let mut fb = Framebuffer {
            file,
            width: if width > 0 { width } else { RM2FB_WIDTH },
            height: if height > 0 { height } else { RM2FB_HEIGHT },
            bits_per_pixel: if bpp > 0 { bpp } else { RM2FB_BPP },
            line_length: if line_length > 0 { line_length } else { RM2FB_WIDTH * 2 },
            mem: fb0_mem as *mut u8,
            mem_len: fb0_mem_len,
            fb0_mem: fb0_mem as *mut u8,
            fb0_mem_len,
            rm2fb_mem: ptr::null_mut(),
            rm2fb_mem_len: 0,
            update_marker: 1,
            partial_refresh_count: 0,
            backend: DisplayBackend::None,
        };

        // Probe display backends in order of preference:
        // 1. Native MXCFB ioctls (cheapest, works on RM1)
        // 2. rm2fb shared memory + message queue (RM2 with rm2fb server)
        // 3. FBIOPAN_DISPLAY fallback (RM2 without rm2fb — degraded quality)

        // Try native ioctls first (skip the probe if we know it's an RM2 driver
        // to avoid spurious error logs, but still try in case a future firmware
        // re-enables it)
        fb.try_auto_update_mode();
        if fb.probe_native_ioctl() {
            return Ok(fb);
        }

        // Native ioctls failed — try rm2fb
        if fb.probe_rm2fb() {
            return Ok(fb);
        }

        // rm2fb not available — try FBIOPAN_DISPLAY fallback
        if fb.probe_fb_pan() {
            return Ok(fb);
        }

        // Nothing works — give detailed guidance
        eprintln!();
        eprintln!("=== NO WORKING DISPLAY BACKEND ===");
        eprintln!();
        eprintln!("  Device:   {}", device_model);
        if let Some(ref ver) = firmware_ver {
            eprintln!("  Firmware: {}", ver);
        }
        eprintln!("  Driver:   {}", fb_id);
        eprintln!();
        if device_model == "reMarkable 2" {
            eprintln!("  The reMarkable 2 requires rm2fb for display refresh.");
            eprintln!("  Native MXCFB ioctls don't work (the kernel driver is mxsfb,");
            eprintln!("  not mxc_epdc_fb). This is expected on RM2.");
            eprintln!();
            eprintln!("  To fix:");
            eprintln!("  1. Download rm2fb for your firmware version from:");
            eprintln!("     https://github.com/ddvk/remarkable2-framebuffer/releases");
            eprintln!("  2. Copy librm2fb_server.so to the tablet");
            eprintln!("  3. Restart xochitl with the server loaded:");
            eprintln!("     systemctl stop xochitl");
            eprintln!("     LD_PRELOAD=/opt/lib/librm2fb_server.so.1 xochitl &");
            eprintln!("  4. Re-run this app");
            eprintln!();
            if let Some(ref ver) = firmware_ver {
                eprintln!("  IMPORTANT: rm2fb must match your firmware ({}).", ver);
                eprintln!("  If you see 'Missing address for function', the rm2fb build");
                eprintln!("  is not compatible with this firmware version.");
            }
        } else {
            eprintln!("  The app will run but the screen will not update.");
            eprintln!("  See README.md for display setup instructions.");
        }
        eprintln!();
        Ok(fb)
    }

    /// Returns true if we're using the rm2fb backend (meaning xochitl should NOT be stopped).
    pub fn is_rm2fb(&self) -> bool {
        matches!(self.backend, DisplayBackend::Rm2fb { .. })
    }

    fn try_auto_update_mode(&self) {
        let fd = self.file.as_raw_fd();
        let mode: u32 = 1;
        let ret = unsafe { sys::ioctl(fd, sys::MXCFB_SET_AUTO_UPDATE_MODE, &mode) };
        if ret == 0 {
            eprintln!("MXCFB_SET_AUTO_UPDATE_MODE: enabled");
        } else {
            eprintln!("MXCFB_SET_AUTO_UPDATE_MODE: not supported (errno {})", sys::errno());
        }
    }

    /// Probe native V2/V1 MXCFB ioctls. Returns true if one works.
    fn probe_native_ioctl(&mut self) -> bool {
        let fd = self.file.as_raw_fd();

        // Try V2
        let update_v2 = MxcfbUpdateData {
            update_region: MxcfbRect { top: 0, left: 0, width: 1, height: 1 },
            waveform_mode: WAVEFORM_MODE_INIT,
            update_mode: UPDATE_MODE_FULL,
            update_marker: 0,
            temp: TEMP_USE_AMBIENT,
            flags: 0,
            dither_mode: 0,
            quant_bit: 0,
            alt_buffer_data: [0; 28],
        };
        let ret = unsafe { sys::ioctl(fd, sys::MXCFB_SEND_UPDATE, &update_v2) };
        if ret == 0 {
            self.backend = DisplayBackend::NativeV2;
            eprintln!("Display backend: native MXCFB V2 ioctl");
            return true;
        }
        eprintln!("MXCFB V2 probe: errno {} ({})", sys::errno(), sys::errno_str());

        // Try V1
        let update_v1 = MxcfbUpdateDataV1 {
            update_region: MxcfbRect { top: 0, left: 0, width: 1, height: 1 },
            waveform_mode: WAVEFORM_MODE_INIT,
            update_mode: UPDATE_MODE_FULL,
            update_marker: 0,
            temp: TEMP_USE_AMBIENT,
            flags: 0,
        };
        let ret = unsafe { sys::ioctl(fd, sys::MXCFB_SEND_UPDATE_V1, &update_v1) };
        if ret == 0 {
            self.backend = DisplayBackend::NativeV1;
            eprintln!("Display backend: native MXCFB V1 ioctl");
            return true;
        }
        eprintln!("MXCFB V1 probe: errno {} ({})", sys::errno(), sys::errno_str());

        false
    }

    /// Try to detect and connect to rm2fb.
    /// rm2fb uses: shared memory at /dev/shm/swtfb.01 + Sys V message queue key 0x2257c.
    /// Returns true if rm2fb is available and we switched to it.
    fn probe_rm2fb(&mut self) -> bool {
        // Check if the shared memory file exists
        let shm_path = std::ffi::CString::new(RM2FB_SHM_PATH).unwrap();
        let shm_fd = unsafe {
            sys::open(shm_path.as_ptr(), sys::O_RDWR)
        };
        if shm_fd < 0 {
            eprintln!("rm2fb probe: {} not found (not running)", RM2FB_SHM_PATH);
            return false;
        }

        // mmap the shared memory
        let shm_mem = unsafe {
            sys::mmap(
                ptr::null_mut(),
                RM2FB_SHM_SIZE,
                sys::PROT_READ | sys::PROT_WRITE,
                sys::MAP_SHARED,
                shm_fd,
                0,
            )
        };
        unsafe { sys::close(shm_fd); }

        if shm_mem == sys::MAP_FAILED {
            eprintln!("rm2fb probe: failed to mmap {} (errno {})", RM2FB_SHM_PATH, sys::errno());
            return false;
        }

        // Try to open the message queue
        let msqid = unsafe { sys::msgget(sys::RM2FB_MSG_KEY, 0) };
        if msqid < 0 {
            eprintln!("rm2fb probe: message queue 0x{:x} not found (errno {})", sys::RM2FB_MSG_KEY, sys::errno());
            // Clean up the shm mmap
            unsafe { sys::munmap(shm_mem, RM2FB_SHM_SIZE); }
            return false;
        }

        // rm2fb is available — switch pixel memory to shared memory
        self.rm2fb_mem = shm_mem as *mut u8;
        self.rm2fb_mem_len = RM2FB_SHM_SIZE;
        self.mem = self.rm2fb_mem;
        self.mem_len = RM2FB_SHM_SIZE;
        self.width = RM2FB_WIDTH;
        self.height = RM2FB_HEIGHT;
        self.bits_per_pixel = RM2FB_BPP;
        self.line_length = RM2FB_WIDTH * (RM2FB_BPP / 8);
        self.backend = DisplayBackend::Rm2fb { msqid };

        eprintln!("Display backend: rm2fb (native client, no LD_PRELOAD needed)");
        eprintln!("  Shared memory: {} ({}x{} @ {}bpp)", RM2FB_SHM_PATH, RM2FB_WIDTH, RM2FB_HEIGHT, RM2FB_BPP);
        eprintln!("  Message queue: id={} key=0x{:x}", msqid, sys::RM2FB_MSG_KEY);

        true
    }

    /// Try FBIOPAN_DISPLAY as a last-resort fallback.
    /// On RM2, the mxsfb driver accepts FBIOPAN_DISPLAY to flip display buffers.
    /// This won't do proper e-ink waveform processing (no grayscale transitions),
    /// but it can push raw pixel data to the panel — resulting in faint/degraded
    /// but potentially usable output.
    fn probe_fb_pan(&mut self) -> bool {
        let fd = self.file.as_raw_fd();

        // Unblank the display first
        let ret = unsafe { sys::ioctl(fd, sys::FBIO_BLANK, sys::FB_BLANK_UNBLANK as sys::c_ulong) };
        if ret < 0 {
            eprintln!("FBIO_BLANK probe: errno {} ({})", sys::errno(), sys::errno_str());
        }

        // Try FBIOPAN_DISPLAY with current vinfo
        let mut vinfo = FbVarScreeninfo::default();
        let ret = unsafe { sys::ioctl(fd, sys::FBIOGET_VSCREENINFO, &mut vinfo) };
        if ret < 0 {
            eprintln!("FBIOPAN probe: can't read vscreeninfo");
            return false;
        }

        // Set offsets to 0 and try panning
        vinfo.xoffset = 0;
        vinfo.yoffset = 0;
        let ret = unsafe { sys::ioctl(fd, sys::FBIOPAN_DISPLAY, &vinfo) };
        if ret < 0 {
            eprintln!("FBIOPAN_DISPLAY probe: errno {} ({})", sys::errno(), sys::errno_str());
            return false;
        }

        self.backend = DisplayBackend::FbPan;
        eprintln!("Display backend: FBIOPAN_DISPLAY (fallback — degraded e-ink quality)");
        eprintln!("  WARNING: Without rm2fb, display updates bypass e-ink waveform");
        eprintln!("  processing. Text may appear faint or require multiple refreshes.");
        eprintln!("  For best results, install rm2fb (see README.md).");
        true
    }

    /// Push update via FBIOPAN_DISPLAY. This tells the mxsfb LCD controller
    /// to re-scan the framebuffer memory. On RM2, this is how SWTCON pushes
    /// frames internally, but without the waveform processing step.
    fn send_update_fb_pan(&mut self) {
        let fd = self.file.as_raw_fd();
        let mut vinfo = FbVarScreeninfo::default();
        let ret = unsafe { sys::ioctl(fd, sys::FBIOGET_VSCREENINFO, &mut vinfo) };
        if ret < 0 {
            return;
        }
        vinfo.xoffset = 0;
        vinfo.yoffset = 0;
        // Activate = set the yoffset and pan
        unsafe { sys::ioctl(fd, sys::FBIOPAN_DISPLAY, &vinfo); }
    }

    /// Send update via rm2fb message queue.
    fn send_update_rm2fb(
        &mut self,
        msqid: sys::c_int,
        x: u32, y: u32, w: u32, h: u32,
        waveform: u32, full: bool,
    ) {
        self.update_marker += 1;
        let msg = SwtfbUpdateMsg {
            mtype: 1, // must be > 0
            update: MxcfbUpdateData {
                update_region: MxcfbRect {
                    top: y,
                    left: x,
                    width: w,
                    height: h,
                },
                waveform_mode: waveform,
                update_mode: if full { UPDATE_MODE_FULL } else { UPDATE_MODE_PARTIAL },
                update_marker: self.update_marker,
                temp: TEMP_USE_AMBIENT,
                flags: 0,
                dither_mode: 0,
                quant_bit: 0,
                alt_buffer_data: [0; 28],
            },
        };

        let data_size = std::mem::size_of::<MxcfbUpdateData>();
        let ret = unsafe {
            sys::msgsnd(
                msqid,
                &msg as *const SwtfbUpdateMsg as *const sys::c_void,
                data_size,
                sys::IPC_NOWAIT,
            )
        };
        if ret < 0 {
            // EAGAIN = queue full, just drop the update (e-ink is slow anyway)
            let e = sys::errno();
            if e != 11 { // EAGAIN
                eprintln!("rm2fb msgsnd failed: errno {} ({})", e, sys::errno_str());
            }
        }
    }

    fn send_update_v2(
        &mut self,
        x: u32, y: u32, w: u32, h: u32,
        waveform: u32, full: bool,
    ) {
        self.update_marker += 1;
        let update = MxcfbUpdateData {
            update_region: MxcfbRect { top: y, left: x, width: w, height: h },
            waveform_mode: waveform,
            update_mode: if full { UPDATE_MODE_FULL } else { UPDATE_MODE_PARTIAL },
            update_marker: self.update_marker,
            temp: TEMP_USE_AMBIENT,
            flags: 0,
            dither_mode: 0,
            quant_bit: 0,
            alt_buffer_data: [0; 28],
        };
        let fd = self.file.as_raw_fd();
        unsafe { sys::ioctl(fd, sys::MXCFB_SEND_UPDATE, &update); }
    }

    fn send_update_v1(
        &mut self,
        x: u32, y: u32, w: u32, h: u32,
        waveform: u32, full: bool,
    ) {
        self.update_marker += 1;
        let update = MxcfbUpdateDataV1 {
            update_region: MxcfbRect { top: y, left: x, width: w, height: h },
            waveform_mode: waveform,
            update_mode: if full { UPDATE_MODE_FULL } else { UPDATE_MODE_PARTIAL },
            update_marker: self.update_marker,
            temp: TEMP_USE_AMBIENT,
            flags: 0,
        };
        let fd = self.file.as_raw_fd();
        unsafe { sys::ioctl(fd, sys::MXCFB_SEND_UPDATE_V1, &update); }
    }

    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, white: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        let bpp_bytes = self.bits_per_pixel / 8;
        let offset = y * self.line_length + x * bpp_bytes;
        if offset + bpp_bytes > self.mem_len {
            return;
        }
        unsafe {
            match self.bits_per_pixel {
                16 => {
                    let val: u16 = if white { 0xFFFF } else { 0x0000 };
                    *(self.mem.add(offset) as *mut u16) = val;
                }
                8 => {
                    *self.mem.add(offset) = if white { 0xFF } else { 0x00 };
                }
                32 => {
                    let val: u32 = if white { 0xFFFFFFFF } else { 0x00000000 };
                    *(self.mem.add(offset) as *mut u32) = val;
                }
                _ => {}
            }
        }
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, white: bool) {
        for row in y..std::cmp::min(y + h, self.height) {
            for col in x..std::cmp::min(x + w, self.width) {
                self.set_pixel(col, row, white);
            }
        }
    }

    pub fn clear(&mut self) {
        unsafe {
            ptr::write_bytes(self.mem, 0xFF, self.mem_len);
        }
    }

    pub fn draw_char(&mut self, ch: u8, x: usize, y: usize, scale: usize, inverse: bool) {
        let glyph = font::glyph(ch);
        for row in 0..font::FONT_HEIGHT {
            let byte = glyph[row];
            for col in 0..font::FONT_WIDTH {
                let on = (byte >> (7 - col)) & 1 == 1;
                let white = if inverse { on } else { !on };
                for sy in 0..scale {
                    for sx in 0..scale {
                        self.set_pixel(
                            x + col * scale + sx,
                            y + row * scale + sy,
                            white,
                        );
                    }
                }
            }
        }
    }

    pub fn draw_str(&mut self, s: &str, x: usize, y: usize, scale: usize, inverse: bool) {
        let char_w = font::FONT_WIDTH * scale;
        for (i, ch) in s.bytes().enumerate() {
            self.draw_char(ch, x + i * char_w, y, scale, inverse);
        }
    }

    pub fn draw_hline(&mut self, x: usize, y: usize, w: usize) {
        for col in x..std::cmp::min(x + w, self.width) {
            self.set_pixel(col, y, false);
            if y + 1 < self.height {
                self.set_pixel(col, y + 1, false);
            }
        }
    }

    /// Returns true if we're using the FBIOPAN_DISPLAY fallback (degraded quality).
    pub fn is_fb_pan(&self) -> bool {
        matches!(self.backend, DisplayBackend::FbPan)
    }

    /// Returns a human-readable name for the current display backend.
    pub fn backend_name(&self) -> &'static str {
        match self.backend {
            DisplayBackend::NativeV2 => "native MXCFB V2",
            DisplayBackend::NativeV1 => "native MXCFB V1",
            DisplayBackend::Rm2fb { .. } => "rm2fb",
            DisplayBackend::FbPan => "FBIOPAN (degraded)",
            DisplayBackend::None => "none",
        }
    }

    pub fn refresh_region(
        &mut self,
        x: u32, y: u32, w: u32, h: u32,
        waveform: u32, full: bool,
    ) {
        match self.backend {
            DisplayBackend::NativeV2 => self.send_update_v2(x, y, w, h, waveform, full),
            DisplayBackend::NativeV1 => self.send_update_v1(x, y, w, h, waveform, full),
            DisplayBackend::Rm2fb { msqid } => self.send_update_rm2fb(msqid, x, y, w, h, waveform, full),
            DisplayBackend::FbPan => self.send_update_fb_pan(),
            DisplayBackend::None => {}
        }
    }

    pub fn refresh_full(&mut self) {
        self.partial_refresh_count = 0;
        self.refresh_region(
            0, 0, self.width as u32, self.height as u32,
            WAVEFORM_MODE_GC16, true,
        );
    }

    pub fn refresh_fast(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.partial_refresh_count += 1;
        if self.partial_refresh_count >= 20 {
            self.refresh_full();
        } else {
            self.refresh_region(x, y, w, h, WAVEFORM_MODE_DU, false);
        }
    }

    pub fn refresh_terminal(&mut self, term_height_px: u32) {
        self.refresh_fast(0, 0, self.width as u32, term_height_px);
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        unsafe {
            // Always unmap /dev/fb0
            if !self.fb0_mem.is_null() {
                sys::munmap(self.fb0_mem as *mut sys::c_void, self.fb0_mem_len);
            }
            // Unmap rm2fb shared memory if we used it
            if !self.rm2fb_mem.is_null() {
                sys::munmap(self.rm2fb_mem as *mut sys::c_void, self.rm2fb_mem_len);
            }
        }
    }
}
