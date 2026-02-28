/// Physical keyboard handling (reMarkable Type Folio) and optional on-screen keyboard.

use std::fs::File;
use std::io::Read;
use std::os::unix::io::{AsRawFd, RawFd};

use crate::font;
use crate::framebuffer::Framebuffer;
use crate::sys;

// ---- Linux key codes (from linux/input-event-codes.h) ----

const KEY_ESC: u16 = 1;
const KEY_1: u16 = 2;
const KEY_2: u16 = 3;
const KEY_3: u16 = 4;
const KEY_4: u16 = 5;
const KEY_5: u16 = 6;
const KEY_6: u16 = 7;
const KEY_7: u16 = 8;
const KEY_8: u16 = 9;
const KEY_9: u16 = 10;
const KEY_0: u16 = 11;
const KEY_MINUS: u16 = 12;
const KEY_EQUAL: u16 = 13;
const KEY_BACKSPACE: u16 = 14;
const KEY_TAB: u16 = 15;
const KEY_Q: u16 = 16;
const KEY_W: u16 = 17;
const KEY_E: u16 = 18;
const KEY_R: u16 = 19;
const KEY_T: u16 = 20;
const KEY_Y: u16 = 21;
const KEY_U: u16 = 22;
const KEY_I: u16 = 23;
const KEY_O: u16 = 24;
const KEY_P: u16 = 25;
const KEY_LEFTBRACE: u16 = 26;
const KEY_RIGHTBRACE: u16 = 27;
const KEY_ENTER: u16 = 28;
const KEY_LEFTCTRL: u16 = 29;
const KEY_A: u16 = 30;
const KEY_S: u16 = 31;
const KEY_D: u16 = 32;
const KEY_F: u16 = 33;
const KEY_G: u16 = 34;
const KEY_H: u16 = 35;
const KEY_J: u16 = 36;
const KEY_K: u16 = 37;
const KEY_L: u16 = 38;
const KEY_SEMICOLON: u16 = 39;
const KEY_APOSTROPHE: u16 = 40;
const KEY_GRAVE: u16 = 41;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_BACKSLASH: u16 = 43;
const KEY_Z: u16 = 44;
const KEY_X: u16 = 45;
const KEY_C: u16 = 46;
const KEY_V: u16 = 47;
const KEY_B: u16 = 48;
const KEY_N: u16 = 49;
const KEY_M: u16 = 50;
const KEY_COMMA: u16 = 51;
const KEY_DOT: u16 = 52;
const KEY_SLASH: u16 = 53;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_LEFTALT: u16 = 56;
const KEY_SPACE: u16 = 57;
const KEY_CAPSLOCK: u16 = 58;
const KEY_F1: u16 = 59;
const KEY_F2: u16 = 60;
const KEY_F3: u16 = 61;
const KEY_F4: u16 = 62;
const KEY_F5: u16 = 63;
const KEY_F6: u16 = 64;
const KEY_F7: u16 = 65;
const KEY_F8: u16 = 66;
const KEY_F9: u16 = 67;
const KEY_F10: u16 = 68;
const KEY_F11: u16 = 87;
const KEY_F12: u16 = 88;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTALT: u16 = 100;
const KEY_HOME: u16 = 102;
const KEY_UP: u16 = 103;
const KEY_PAGEUP: u16 = 104;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_END: u16 = 107;
const KEY_DOWN: u16 = 108;
const KEY_PAGEDOWN: u16 = 109;
const KEY_INSERT: u16 = 110;
const KEY_DELETE: u16 = 111;

// evdev event type
const EV_KEY: u16 = 0x01;

/// Raw Linux input_event structure
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct InputEvent {
    tv_sec: sys::time_t,
    tv_usec: sys::suseconds_t,
    type_: u16,
    code: u16,
    value: i32,
}

#[derive(Default)]
struct Modifiers {
    shift: bool,
    ctrl: bool,
    alt: bool,
    caps_lock: bool,
}

// ioctl numbers for evdev queries

/// Compute EVIOCGNAME(len) ioctl number
fn eviocgname(len: usize) -> sys::c_ulong {
    // _IOC(_IOC_READ, 'E', 0x06, len) for ARM/x86
    // direction=2 (read), type='E'=0x45, nr=0x06, size=len
    (2 << 30) | ((len as sys::c_ulong) << 16) | (0x45 << 8) | 0x06
}

/// Compute EVIOCGBIT(ev_type, len) ioctl number
fn eviocgbit(ev_type: u16, len: usize) -> sys::c_ulong {
    // _IOC(_IOC_READ, 'E', 0x20 + ev_type, len)
    (2 << 30) | ((len as sys::c_ulong) << 16) | (0x45 << 8) | (0x20 + ev_type as sys::c_ulong)
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

/// Check if an evdev device looks like a keyboard by querying its capabilities.
/// Returns true if the device supports KEY_A through KEY_Z and KEY_ENTER.
fn is_keyboard_device(fd: sys::c_int) -> bool {
    // Query EV_KEY capability bits - we need enough bytes for key code 111 (KEY_DELETE)
    // That's ceil(112/8) = 14 bytes minimum, use 64 to be safe
    let mut key_bits = [0u8; 64];
    let ret = unsafe {
        sys::ioctl(fd, eviocgbit(EV_KEY, key_bits.len()), key_bits.as_mut_ptr())
    };
    if ret < 0 {
        return false;
    }

    // Must support KEY_A(30)..KEY_Z(44+) and KEY_ENTER(28)
    let required_keys = [
        KEY_A, KEY_B, KEY_C, KEY_D, KEY_E, KEY_F, KEY_G, KEY_H, KEY_I,
        KEY_J, KEY_K, KEY_L, KEY_M, KEY_N, KEY_O, KEY_P, KEY_Q, KEY_R,
        KEY_S, KEY_T, KEY_U, KEY_V, KEY_W, KEY_X, KEY_Y, KEY_Z, KEY_ENTER,
    ];

    for &key in &required_keys {
        if !bit_is_set(&key_bits, key) {
            return false;
        }
    }

    true
}

/// Get device name via EVIOCGNAME ioctl
fn get_device_name(fd: sys::c_int) -> Option<String> {
    let mut name_buf = [0u8; 256];
    let ret = unsafe {
        sys::ioctl(fd, eviocgname(name_buf.len()), name_buf.as_mut_ptr())
    };
    if ret > 0 {
        let len = (ret as usize).min(name_buf.len());
        // Find null terminator
        let end = name_buf[..len].iter().position(|&b| b == 0).unwrap_or(len);
        Some(String::from_utf8_lossy(&name_buf[..end]).to_string())
    } else {
        None
    }
}

pub struct PhysicalKeyboard {
    file: Option<File>,
    mods: Modifiers,
}

impl PhysicalKeyboard {
    /// Open a specific keyboard device by path.
    pub fn open_path(path: &str) -> Self {
        let file = match File::open(path) {
            Ok(f) => {
                eprintln!("Opened keyboard device: {}", path);
                Some(f)
            }
            Err(e) => {
                eprintln!("Warning: Cannot open keyboard device {}: {}", path, e);
                None
            }
        };
        PhysicalKeyboard {
            file,
            mods: Modifiers::default(),
        }
    }

    /// Auto-discover a keyboard device.
    /// 1. Try /dev/input/by-id/*kbd* or /dev/input/by-path/*kbd*
    /// 2. Scan /dev/input/event* and probe with EVIOCGBIT for key capabilities
    /// 3. Fall back to the old hardcoded order
    pub fn open() -> Self {
        // Strategy 1: Try by-id and by-path symlinks containing "kbd"
        for dir in &["/dev/input/by-id", "/dev/input/by-path"] {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.contains("kbd") || name_str.contains("keyboard") {
                        let path = entry.path();
                        if let Ok(f) = File::open(&path) {
                            eprintln!("Opened keyboard device (by-id/path): {}", path.display());
                            return PhysicalKeyboard {
                                file: Some(f),
                                mods: Modifiers::default(),
                            };
                        }
                    }
                }
            }
        }

        // Strategy 2: Scan /dev/input/event* and probe capabilities
        for i in 0..20 {
            let path = format!("/dev/input/event{}", i);
            if let Ok(f) = File::open(&path) {
                let fd = f.as_raw_fd();
                if is_keyboard_device(fd) {
                    let name = get_device_name(fd).unwrap_or_default();
                    eprintln!("Auto-detected keyboard: {} ({})", path, name);
                    return PhysicalKeyboard {
                        file: Some(f),
                        mods: Modifiers::default(),
                    };
                }
            }
        }

        // Strategy 3: Legacy fallback
        let paths = [
            "/dev/input/event3",
            "/dev/input/event4",
            "/dev/input/event2",
            "/dev/input/event5",
        ];

        let mut file = None;
        for path in &paths {
            if let Ok(f) = File::open(path) {
                eprintln!("Opened keyboard device (fallback): {}", path);
                file = Some(f);
                break;
            }
        }

        if file.is_none() {
            eprintln!("Warning: No physical keyboard device found");
        }

        PhysicalKeyboard {
            file,
            mods: Modifiers::default(),
        }
    }

    pub fn fd(&self) -> Option<RawFd> {
        self.file.as_ref().map(|f| f.as_raw_fd())
    }

    pub fn read_keys(&mut self, app_cursor_keys: bool) -> Vec<Vec<u8>> {
        let mut results = Vec::new();

        let file = match &mut self.file {
            Some(f) => f,
            None => return results,
        };

        let mut buf = [0u8; std::mem::size_of::<InputEvent>() * 16];
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return results,
        };

        let event_size = std::mem::size_of::<InputEvent>();
        let count = n / event_size;

        for i in 0..count {
            let ev: InputEvent = unsafe {
                std::ptr::read_unaligned(buf.as_ptr().add(i * event_size) as *const InputEvent)
            };

            if ev.type_ != EV_KEY {
                continue;
            }

            // value: 0=release, 1=press, 2=repeat
            if ev.value == 0 {
                match ev.code {
                    KEY_LEFTSHIFT | KEY_RIGHTSHIFT => self.mods.shift = false,
                    KEY_LEFTCTRL | KEY_RIGHTCTRL => self.mods.ctrl = false,
                    KEY_LEFTALT | KEY_RIGHTALT => self.mods.alt = false,
                    _ => {}
                }
                continue;
            }

            match ev.code {
                KEY_LEFTSHIFT | KEY_RIGHTSHIFT => {
                    self.mods.shift = true;
                    continue;
                }
                KEY_LEFTCTRL | KEY_RIGHTCTRL => {
                    self.mods.ctrl = true;
                    continue;
                }
                KEY_LEFTALT | KEY_RIGHTALT => {
                    self.mods.alt = true;
                    continue;
                }
                KEY_CAPSLOCK => {
                    if ev.value == 1 {
                        self.mods.caps_lock = !self.mods.caps_lock;
                    }
                    continue;
                }
                _ => {}
            }

            if let Some(bytes) = self.map_key(ev.code, app_cursor_keys) {
                results.push(bytes);
            }
        }

        results
    }

    fn map_key(&self, code: u16, app_cursor_keys: bool) -> Option<Vec<u8>> {
        match code {
            KEY_ESC => return Some(vec![0x1B]),
            KEY_TAB => return Some(vec![0x09]),
            KEY_ENTER => return Some(vec![0x0D]),
            KEY_BACKSPACE => {
                return Some(if self.mods.ctrl {
                    vec![0x08]
                } else {
                    vec![0x7F]
                });
            }
            KEY_DELETE => return Some(b"\x1B[3~".to_vec()),
            KEY_INSERT => return Some(b"\x1B[2~".to_vec()),
            KEY_HOME => return Some(b"\x1B[H".to_vec()),
            KEY_END => return Some(b"\x1B[F".to_vec()),
            KEY_PAGEUP => return Some(b"\x1B[5~".to_vec()),
            KEY_PAGEDOWN => return Some(b"\x1B[6~".to_vec()),
            KEY_UP => {
                return Some(if app_cursor_keys {
                    b"\x1BOA".to_vec()
                } else {
                    b"\x1B[A".to_vec()
                });
            }
            KEY_DOWN => {
                return Some(if app_cursor_keys {
                    b"\x1BOB".to_vec()
                } else {
                    b"\x1B[B".to_vec()
                });
            }
            KEY_RIGHT => {
                return Some(if app_cursor_keys {
                    b"\x1BOC".to_vec()
                } else {
                    b"\x1B[C".to_vec()
                });
            }
            KEY_LEFT => {
                return Some(if app_cursor_keys {
                    b"\x1BOD".to_vec()
                } else {
                    b"\x1B[D".to_vec()
                });
            }
            KEY_F1 => return Some(b"\x1BOP".to_vec()),
            KEY_F2 => return Some(b"\x1BOQ".to_vec()),
            KEY_F3 => return Some(b"\x1BOR".to_vec()),
            KEY_F4 => return Some(b"\x1BOS".to_vec()),
            KEY_F5 => return Some(b"\x1B[15~".to_vec()),
            KEY_F6 => return Some(b"\x1B[17~".to_vec()),
            KEY_F7 => return Some(b"\x1B[18~".to_vec()),
            KEY_F8 => return Some(b"\x1B[19~".to_vec()),
            KEY_F9 => return Some(b"\x1B[20~".to_vec()),
            KEY_F10 => return Some(b"\x1B[21~".to_vec()),
            KEY_F11 => return Some(b"\x1B[23~".to_vec()),
            KEY_F12 => return Some(b"\x1B[24~".to_vec()),
            _ => {}
        }

        let (normal, shifted) = match code {
            KEY_1 => (b'1', b'!'),
            KEY_2 => (b'2', b'@'),
            KEY_3 => (b'3', b'#'),
            KEY_4 => (b'4', b'$'),
            KEY_5 => (b'5', b'%'),
            KEY_6 => (b'6', b'^'),
            KEY_7 => (b'7', b'&'),
            KEY_8 => (b'8', b'*'),
            KEY_9 => (b'9', b'('),
            KEY_0 => (b'0', b')'),
            KEY_MINUS => (b'-', b'_'),
            KEY_EQUAL => (b'=', b'+'),
            KEY_Q => (b'q', b'Q'),
            KEY_W => (b'w', b'W'),
            KEY_E => (b'e', b'E'),
            KEY_R => (b'r', b'R'),
            KEY_T => (b't', b'T'),
            KEY_Y => (b'y', b'Y'),
            KEY_U => (b'u', b'U'),
            KEY_I => (b'i', b'I'),
            KEY_O => (b'o', b'O'),
            KEY_P => (b'p', b'P'),
            KEY_A => (b'a', b'A'),
            KEY_S => (b's', b'S'),
            KEY_D => (b'd', b'D'),
            KEY_F => (b'f', b'F'),
            KEY_G => (b'g', b'G'),
            KEY_H => (b'h', b'H'),
            KEY_J => (b'j', b'J'),
            KEY_K => (b'k', b'K'),
            KEY_L => (b'l', b'L'),
            KEY_Z => (b'z', b'Z'),
            KEY_X => (b'x', b'X'),
            KEY_C => (b'c', b'C'),
            KEY_V => (b'v', b'V'),
            KEY_B => (b'b', b'B'),
            KEY_N => (b'n', b'N'),
            KEY_M => (b'm', b'M'),
            KEY_SEMICOLON => (b';', b':'),
            KEY_APOSTROPHE => (b'\'', b'"'),
            KEY_GRAVE => (b'`', b'~'),
            KEY_BACKSLASH => (b'\\', b'|'),
            KEY_LEFTBRACE => (b'[', b'{'),
            KEY_RIGHTBRACE => (b']', b'}'),
            KEY_COMMA => (b',', b'<'),
            KEY_DOT => (b'.', b'>'),
            KEY_SLASH => (b'/', b'?'),
            KEY_SPACE => (b' ', b' '),
            _ => return None,
        };

        let is_letter = matches!(
            code,
            KEY_Q..=KEY_P | KEY_A..=KEY_L | KEY_Z..=KEY_M
        );

        let mut ch = if is_letter {
            if self.mods.shift ^ self.mods.caps_lock {
                shifted
            } else {
                normal
            }
        } else if self.mods.shift {
            shifted
        } else {
            normal
        };

        if self.mods.ctrl {
            if ch >= b'a' && ch <= b'z' {
                ch = ch - b'a' + 1;
            } else if ch >= b'A' && ch <= b'Z' {
                ch = ch - b'A' + 1;
            } else {
                match ch {
                    b'[' => ch = 0x1B,
                    b'\\' => ch = 0x1C,
                    b']' => ch = 0x1D,
                    b'^' => ch = 0x1E,
                    b'_' => ch = 0x1F,
                    b' ' => ch = 0x00,
                    _ => {}
                }
            }
        }

        if self.mods.alt {
            return Some(vec![0x1B, ch]);
        }

        Some(vec![ch])
    }
}

// ---- Optional On-Screen Keyboard ----

struct VKey {
    label: &'static str,
    shifted_label: &'static str,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    output: &'static [u8],
    shifted_output: &'static [u8],
}

pub struct OnScreenKeyboard {
    keys: Vec<Vec<VKey>>,
    pub origin_x: usize,
    pub origin_y: usize,
    pub total_width: usize,
    pub total_height: usize,
    shift: bool,
    ctrl: bool,
}

impl OnScreenKeyboard {
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        let key_h = height / 5;
        let key_w = width / 12;

        let rows_data: Vec<Vec<(&str, &str, &[u8], &[u8], usize)>> = vec![
            vec![
                ("1","!",b"1",b"!",1), ("2","@",b"2",b"@",1), ("3","#",b"3",b"#",1),
                ("4","$",b"4",b"$",1), ("5","%",b"5",b"%",1), ("6","^",b"6",b"^",1),
                ("7","&",b"7",b"&",1), ("8","*",b"8",b"*",1), ("9","(",b"9",b"(",1),
                ("0",")",b"0",b")",1), ("-","_",b"-",b"_",1), ("=","+",b"=",b"+",1),
            ],
            vec![
                ("q","Q",b"q",b"Q",1), ("w","W",b"w",b"W",1), ("e","E",b"e",b"E",1),
                ("r","R",b"r",b"R",1), ("t","T",b"t",b"T",1), ("y","Y",b"y",b"Y",1),
                ("u","U",b"u",b"U",1), ("i","I",b"i",b"I",1), ("o","O",b"o",b"O",1),
                ("p","P",b"p",b"P",1), ("[","{",b"[",b"{",1), ("]","}",b"]",b"}",1),
            ],
            vec![
                ("a","A",b"a",b"A",1), ("s","S",b"s",b"S",1), ("d","D",b"d",b"D",1),
                ("f","F",b"f",b"F",1), ("g","G",b"g",b"G",1), ("h","H",b"h",b"H",1),
                ("j","J",b"j",b"J",1), ("k","K",b"k",b"K",1), ("l","L",b"l",b"L",1),
                (";",":",b";",b":",1), ("'","\"",b"'",b"\"",1), ("\\","|",b"\\",b"|",1),
            ],
            vec![
                ("SH","SH",b"",b"",1), ("z","Z",b"z",b"Z",1), ("x","X",b"x",b"X",1),
                ("c","C",b"c",b"C",1), ("v","V",b"v",b"V",1), ("b","B",b"b",b"B",1),
                ("n","N",b"n",b"N",1), ("m","M",b"m",b"M",1), (",","<",b",",b"<",1),
                (".",">",b".",b">",1), ("/","?",b"/",b"?",1), ("BS","BS",b"\x7f",b"\x7f",1),
            ],
            vec![
                ("CT","CT",b"",b"",1), ("ES","ES",b"\x1b",b"\x1b",1), ("TB","TB",b"\t",b"\t",1),
                (" "," ",b" ",b" ",5),
                ("<-","<-",b"\x1b[D",b"\x1b[D",1), ("->","->",b"\x1b[C",b"\x1b[C",1),
                ("EN","EN",b"\r",b"\r",2),
            ],
        ];

        let mut keys = Vec::new();
        for (row_idx, row_data) in rows_data.iter().enumerate() {
            let mut row_keys = Vec::new();
            let mut cx = x;
            for &(label, shifted_label, output, shifted_output, wm) in row_data {
                let kw = key_w * wm;
                row_keys.push(VKey {
                    label,
                    shifted_label,
                    x: cx,
                    y: y + row_idx * key_h,
                    w: kw,
                    h: key_h,
                    output,
                    shifted_output,
                });
                cx += kw;
            }
            keys.push(row_keys);
        }

        OnScreenKeyboard {
            keys,
            origin_x: x,
            origin_y: y,
            total_width: width,
            total_height: height,
            shift: false,
            ctrl: false,
        }
    }

    pub fn render(&self, fb: &mut Framebuffer, scale: usize) {
        fb.fill_rect(self.origin_x, self.origin_y, self.total_width, self.total_height, true);
        fb.draw_hline(self.origin_x, self.origin_y, self.total_width);

        for row in &self.keys {
            for key in row {
                for px in key.x..key.x + key.w {
                    fb.set_pixel(px, key.y, false);
                    fb.set_pixel(px, key.y + key.h - 1, false);
                }
                for py in key.y..key.y + key.h {
                    fb.set_pixel(key.x, py, false);
                    fb.set_pixel(key.x + key.w - 1, py, false);
                }

                let label = if self.shift { key.shifted_label } else { key.label };
                let char_w = font::FONT_WIDTH * scale;
                let char_h = font::FONT_HEIGHT * scale;
                let lx = key.x + (key.w.saturating_sub(label.len() * char_w)) / 2;
                let ly = key.y + (key.h.saturating_sub(char_h)) / 2;

                let inverse = (key.label == "SH" && self.shift)
                    || (key.label == "CT" && self.ctrl);
                fb.draw_str(label, lx, ly, scale, inverse);
            }
        }
    }

    pub fn handle_touch(&mut self, x: i32, y: i32) -> Option<Vec<u8>> {
        for row in &self.keys {
            for key in row {
                if x >= key.x as i32
                    && x < (key.x + key.w) as i32
                    && y >= key.y as i32
                    && y < (key.y + key.h) as i32
                {
                    if key.label == "SH" {
                        self.shift = !self.shift;
                        return None;
                    }
                    if key.label == "CT" {
                        self.ctrl = !self.ctrl;
                        return None;
                    }

                    let output = if self.shift {
                        key.shifted_output
                    } else {
                        key.output
                    };

                    if output.is_empty() {
                        return None;
                    }

                    let mut bytes = output.to_vec();

                    if self.ctrl && bytes.len() == 1 {
                        let ch = bytes[0];
                        if ch >= b'a' && ch <= b'z' {
                            bytes[0] = ch - b'a' + 1;
                        } else if ch >= b'A' && ch <= b'Z' {
                            bytes[0] = ch - b'A' + 1;
                        }
                        self.ctrl = false;
                    }

                    if self.shift {
                        self.shift = false;
                    }

                    return Some(bytes);
                }
            }
        }
        None
    }
}
