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

// Event types
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;

// ABS codes for multitouch
const ABS_MT_SLOT: u16 = 0x2F;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_PRESSURE: u16 = 0x3A;

// Button codes
const BTN_TOUCH: u16 = 0x14A;

// Wacom digitizer ABS codes
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_PRESSURE: u16 = 0x18;

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
    single_x: i32,
    single_y: i32,
    single_pressure: i32,
    single_down: bool,
    pub screen_width: i32,
    pub screen_height: i32,
    pub touch_max_x: i32,
    pub touch_max_y: i32,
}

impl Input {
    pub fn open(screen_width: i32, screen_height: i32) -> Result<Self, String> {
        let mut files = Vec::new();

        let paths = [
            "/dev/input/event0",
            "/dev/input/event1",
            "/dev/input/event2",
            "/dev/input/event3",
        ];

        for path in &paths {
            match File::open(path) {
                Ok(f) => {
                    eprintln!("Opened input device: {}", path);
                    files.push(f);
                }
                Err(_) => {}
            }
        }

        if files.is_empty() {
            return Err("No input devices found".to_string());
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
            single_x: 0,
            single_y: 0,
            single_pressure: 0,
            single_down: false,
            screen_width,
            screen_height,
            touch_max_x: 767,
            touch_max_y: 1023,
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
                std::ptr::read(buf.as_ptr().add(i * event_size) as *const InputEvent)
            };

            match ev.type_ {
                EV_ABS => match ev.code {
                    ABS_MT_SLOT => {
                        self.current_slot = ev.value as usize;
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
                    ABS_X => self.single_x = ev.value,
                    ABS_Y => self.single_y = ev.value,
                    ABS_PRESSURE => self.single_pressure = ev.value,
                    _ => {}
                },
                EV_KEY => {
                    if ev.code == BTN_TOUCH {
                        self.single_down = ev.value != 0;
                    }
                }
                EV_SYN => {
                    if ev.code == 0 {
                        if self.slots[0].tracking_id >= 0 {
                            let (mx, my) = self.map_touch(self.slots[0].x, self.slots[0].y);
                            events.push(TouchEvent {
                                x: mx,
                                y: my,
                                pressure: self.slots[0].pressure,
                            });
                        } else if self.single_down {
                            let (mx, my) = self.map_pen(self.single_x, self.single_y);
                            events.push(TouchEvent {
                                x: mx,
                                y: my,
                                pressure: self.single_pressure,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        events
    }

    fn map_touch(&self, raw_x: i32, raw_y: i32) -> (i32, i32) {
        let sx = raw_y * self.screen_width / self.touch_max_y.max(1);
        let sy = (self.touch_max_x - raw_x) * self.screen_height / self.touch_max_x.max(1);
        (
            sx.clamp(0, self.screen_width - 1),
            sy.clamp(0, self.screen_height - 1),
        )
    }

    fn map_pen(&self, raw_x: i32, raw_y: i32) -> (i32, i32) {
        let pen_max_x = 20967;
        let pen_max_y = 15725;
        let sx = raw_x * self.screen_width / pen_max_x;
        let sy = raw_y * self.screen_height / pen_max_y;
        (
            sx.clamp(0, self.screen_width - 1),
            sy.clamp(0, self.screen_height - 1),
        )
    }
}
