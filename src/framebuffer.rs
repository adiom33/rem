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

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MxcfbUpdateData {
    pub update_region: MxcfbRect,
    pub waveform_mode: u32,
    pub update_mode: u32,
    pub update_marker: u32,
    pub temp: u32,
    pub flags: u32,
    pub dither_mode: u32,
    pub quant_bit: i32,
    pub _padding: [u8; 28],
}

// Waveform modes
pub const WAVEFORM_MODE_DU: u32 = 1;
pub const WAVEFORM_MODE_GC16: u32 = 2;

// Update modes
pub const UPDATE_MODE_PARTIAL: u32 = 0;
pub const UPDATE_MODE_FULL: u32 = 1;

// Temperature
pub const TEMP_USE_AMBIENT: u32 = 0x1000;

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

pub struct Framebuffer {
    file: File,
    pub width: usize,
    pub height: usize,
    pub bits_per_pixel: usize,
    line_length: usize,
    mem: *mut u8,
    mem_len: usize,
    update_marker: u32,
    partial_refresh_count: u32,
}

unsafe impl Send for Framebuffer {}

impl Framebuffer {
    pub fn open(path: &str) -> Result<Self, String> {
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
            return Err(format!("FBIOGET_VSCREENINFO failed: {}", sys::errno_str()));
        }

        // Get fixed screen info (for smem_len)
        let mut finfo = FbFixScreeninfo::default();
        let ret = unsafe { sys::ioctl(fd, sys::FBIOGET_FSCREENINFO, &mut finfo) };
        if ret < 0 {
            return Err(format!("FBIOGET_FSCREENINFO failed: {}", sys::errno_str()));
        }

        let width = vinfo.xres as usize;
        let height = vinfo.yres as usize;
        let bpp = vinfo.bits_per_pixel as usize;
        let bytes_per_pixel = bpp / 8;
        let line_length = width * bytes_per_pixel;
        let mem_len = line_length * height;

        eprintln!(
            "Framebuffer: {}x{} @ {}bpp ({}KB)",
            width, height, bpp, mem_len / 1024
        );

        // mmap the framebuffer
        let mem = unsafe {
            sys::mmap(
                ptr::null_mut(),
                mem_len,
                sys::PROT_READ | sys::PROT_WRITE,
                sys::MAP_SHARED,
                fd,
                0,
            )
        };
        if mem == sys::MAP_FAILED {
            return Err(format!("mmap failed: {}", sys::errno_str()));
        }

        Ok(Framebuffer {
            file,
            width,
            height,
            bits_per_pixel: bpp,
            line_length,
            mem: mem as *mut u8,
            mem_len,
            update_marker: 1,
            partial_refresh_count: 0,
        })
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

    pub fn draw_char(
        &mut self,
        ch: u8,
        x: usize,
        y: usize,
        scale: usize,
        inverse: bool,
    ) {
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

    pub fn draw_str(
        &mut self,
        s: &str,
        x: usize,
        y: usize,
        scale: usize,
        inverse: bool,
    ) {
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

    pub fn refresh_region(
        &mut self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        waveform: u32,
        full: bool,
    ) {
        self.update_marker += 1;
        let update = MxcfbUpdateData {
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
            _padding: [0; 28],
        };

        let fd = self.file.as_raw_fd();
        let ret = unsafe { sys::ioctl(fd, sys::MXCFB_SEND_UPDATE, &update) };
        if ret < 0 {
            // Silently ignore on non-reMarkable hardware
        }
    }

    pub fn refresh_full(&mut self) {
        self.partial_refresh_count = 0;
        self.refresh_region(
            0,
            0,
            self.width as u32,
            self.height as u32,
            WAVEFORM_MODE_GC16,
            true,
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
            sys::munmap(self.mem as *mut sys::c_void, self.mem_len);
        }
    }
}
