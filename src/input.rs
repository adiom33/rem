use std::fs::File;
use std::io::Read;
use std::os::unix::io::{AsRawFd, RawFd};

use crate::sys;

/// Raw Linux input_event structure (from linux/input.h)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct InputEvent {
    tv_sec: sys::time_t,
    tv_usec: sys::suseconds_t,
    type_: u16,
    code: u16,
    value: i32,
}

/// Kernel struct input_absinfo (from linux/input.h)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct InputAbsinfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

// Event types
const EV_SYN: u16 = 0x00;
const EV_ABS: u16 = 0x03;

// ABS codes for multitouch
const ABS_MT_SLOT: u16 = 0x2F;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_PRESSURE: u16 = 0x3A;


/// Compute EVIOCGBIT(ev_type, len) ioctl number
fn eviocgbit(ev_type: u16, len: usize) -> sys::c_ulong {
    (2 << 30) | ((len as sys::c_ulong) << 16) | (0x45 << 8) | (0x20 + ev_type as sys::c_ulong)
}

/// Compute EVIOCGABS(axis) ioctl number
fn eviocgabs(axis: u16) -> sys::c_ulong {
    (2 << 30)
        | ((std::mem::size_of::<InputAbsinfo>() as sys::c_ulong) << 16)
        | (0x45 << 8)
        | (0x40 + axis as sys::c_ulong)
}

/// Check if a bit is set in a bitfield array
fn bit_is_set(bits: &[u8], bit: u16) -> bool {
    let byte_idx = (bit / 8) as usize;
    let bit_idx = bit % 8;
    if byte_idx < bits.len() {
        (bits[byte_idx] >> bit_idx) & 1 != 0
    } else {
        false
    }
}

/// Check if a device supports multitouch (ABS_MT_POSITION_X/Y).
/// This filters out the Wacom pen digitizer which only has ABS_X/ABS_Y.
fn is_touch_device(fd: sys::c_int) -> bool {
    let mut abs_bits = [0u8; 8]; // 64 bits, enough for ABS codes up to 0x3F
    let ret = unsafe {
        sys::ioctl(fd, eviocgbit(EV_ABS, abs_bits.len()), abs_bits.as_mut_ptr())
    };
    if ret < 0 {
        return false;
    }
    // Must support ABS_MT_POSITION_X (0x35) and ABS_MT_POSITION_Y (0x36)
    bit_is_set(&abs_bits, ABS_MT_POSITION_X) && bit_is_set(&abs_bits, ABS_MT_POSITION_Y)
}

/// Query the maximum value for an ABS axis via EVIOCGABS.
fn get_abs_max(fd: sys::c_int, axis: u16) -> Option<i32> {
    let mut info = InputAbsinfo::default();
    let ret = unsafe {
        sys::ioctl(fd, eviocgabs(axis), &mut info)
    };
    if ret == 0 && info.maximum > 0 {
        Some(info.maximum)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TouchEvent {
    pub x: i32,
    pub y: i32,
    pub pressure: i32,
}

struct TouchSlot {
    x: i32,
    y: i32,
    pressure: i32,
    tracking_id: i32,
}

pub struct Input {
    files: Vec<File>,
    current_slot: usize,
    slots: [TouchSlot; 10],
    pub screen_width: i32,
    pub screen_height: i32,
    pub touch_max_x: i32,
    pub touch_max_y: i32,
    /// Whether the display is in landscape (rotated) mode.
    /// Affects how raw touch coordinates map to screen coordinates.
    pub rotated: bool,
}

impl Input {
    pub fn open(screen_width: i32, screen_height: i32, rotated: bool) -> Result<Self, String> {
        let mut files = Vec::new();
        let mut detected_max_x: Option<i32> = None;
        let mut detected_max_y: Option<i32> = None;

        // Scan /dev/input/event* and only open multitouch devices.
        // This filters out the Wacom pen digitizer, which would otherwise
        // generate false touch events on the on-screen keyboard.
        for i in 0..10 {
            let path = format!("/dev/input/event{}", i);
            match File::open(&path) {
                Ok(f) => {
                    let fd = f.as_raw_fd();
                    if is_touch_device(fd) {
                        eprintln!("Opened touch input: {} (multitouch)", path);
                        // Query actual touch axis ranges from the device
                        if detected_max_x.is_none() {
                            if let Some(mx) = get_abs_max(fd, ABS_MT_POSITION_X) {
                                eprintln!("  ABS_MT_POSITION_X max: {}", mx);
                                detected_max_x = Some(mx);
                            }
                        }
                        if detected_max_y.is_none() {
                            if let Some(my) = get_abs_max(fd, ABS_MT_POSITION_Y) {
                                eprintln!("  ABS_MT_POSITION_Y max: {}", my);
                                detected_max_y = Some(my);
                            }
                        }
                        files.push(f);
                    }
                    // Non-touch devices (pen, buttons) are silently skipped
                }
                Err(_) => {}
            }
        }

        if files.is_empty() {
            return Err("No multitouch input devices found".to_string());
        }

        Ok(Input {
            files,
            current_slot: 0,
            slots: std::array::from_fn(|_| TouchSlot {
                x: 0,
                y: 0,
                pressure: 0,
                tracking_id: -1,
            }),
            screen_width,
            screen_height,
            // Use detected values, fall back to known rM2 defaults
            touch_max_x: detected_max_x.unwrap_or(767),
            touch_max_y: detected_max_y.unwrap_or(1023),
            rotated,
        })
    }

    pub fn fds(&self) -> Vec<RawFd> {
        self.files.iter().map(|f| f.as_raw_fd()).collect()
    }

    pub fn read_events(&mut self, fd_index: usize) -> Vec<TouchEvent> {
        let mut events = Vec::new();
        let mut buf = [0u8; std::mem::size_of::<InputEvent>() * 16];

        let file = match self.files.get_mut(fd_index) {
            Some(f) => f,
            None => return events,
        };

        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return events,
        };

        let event_size = std::mem::size_of::<InputEvent>();
        let count = n / event_size;

        for i in 0..count {
            let ev: InputEvent = unsafe {
                std::ptr::read_unaligned(buf.as_ptr().add(i * event_size) as *const InputEvent)
            };

            match ev.type_ {
                EV_ABS => match ev.code {
                    ABS_MT_SLOT => {
                        self.current_slot = (ev.value as usize).min(self.slots.len() - 1);
                    }
                    ABS_MT_TRACKING_ID => {
                        if self.current_slot < self.slots.len() {
                            self.slots[self.current_slot].tracking_id = ev.value;
                        }
                    }
                    ABS_MT_POSITION_X => {
                        if self.current_slot < self.slots.len() {
                            self.slots[self.current_slot].x = ev.value;
                        }
                    }
                    ABS_MT_POSITION_Y => {
                        if self.current_slot < self.slots.len() {
                            self.slots[self.current_slot].y = ev.value;
                        }
                    }
                    ABS_MT_PRESSURE => {
                        if self.current_slot < self.slots.len() {
                            self.slots[self.current_slot].pressure = ev.value;
                        }
                    }
                    _ => {}
                },
                EV_SYN => {
                    if ev.code == 0 && self.slots[0].tracking_id >= 0 {
                        let (mx, my) = self.map_touch(self.slots[0].x, self.slots[0].y);
                        events.push(TouchEvent {
                            x: mx,
                            y: my,
                            pressure: self.slots[0].pressure,
                        });
                    }
                }
                _ => {}
            }
        }

        events
    }

    fn map_touch(&self, raw_x: i32, raw_y: i32) -> (i32, i32) {
        // The rM2 touch panel is physically rotated 90° relative to the display.
        // In portrait mode: raw X → short axis, raw Y → long axis,
        //   so screen_x = raw_y scaled, screen_y = (max_x - raw_x) scaled.
        // In landscape mode: the framebuffer is also rotated 90° CW,
        //   so screen_x = raw_x scaled, screen_y = raw_y scaled.
        if self.rotated {
            // Landscape: touch panel aligns naturally with the rotated screen
            let sx = raw_x * self.screen_width / self.touch_max_x.max(1);
            let sy = raw_y * self.screen_height / self.touch_max_y.max(1);
            (
                sx.clamp(0, self.screen_width - 1),
                sy.clamp(0, self.screen_height - 1),
            )
        } else {
            // Portrait: compensate for 90° hardware rotation
            let sx = raw_y * self.screen_width / self.touch_max_y.max(1);
            let sy = (self.touch_max_x - raw_x) * self.screen_height / self.touch_max_x.max(1);
            (
                sx.clamp(0, self.screen_width - 1),
                sy.clamp(0, self.screen_height - 1),
            )
        }
    }
}
