/// VT100/xterm terminal emulator.
/// Implements enough escape sequences for interactive SSH sessions:
/// cursor movement, erase, scrolling, basic SGR (bold/inverse).

#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: u8,
    pub bold: bool,
    pub inverse: bool,
    pub dirty: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: b' ',
            bold: false,
            inverse: false,
            dirty: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Normal,
    Escape,     // Got ESC
    CSI,        // Got ESC[
    OSC,        // Got ESC]
    CSIParam,   // Collecting CSI parameters
}

pub struct Terminal {
    pub cols: usize,
    pub rows: usize,
    pub grid: Vec<Vec<Cell>>,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub dirty: bool,

    // Parse state
    state: State,
    params: Vec<u32>,
    current_param: Option<u32>,
    private_mode: bool,

    // Current text attributes
    bold: bool,
    inverse: bool,

    // Scroll region
    scroll_top: usize,
    scroll_bottom: usize,

    // Saved cursor
    saved_x: usize,
    saved_y: usize,

    // Application cursor keys mode (DECCKM)
    pub app_cursor_keys: bool,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        let grid = vec![vec![Cell::default(); cols]; rows];
        Terminal {
            cols,
            rows,
            grid,
            cursor_x: 0,
            cursor_y: 0,
            dirty: true,
            state: State::Normal,
            params: Vec::new(),
            current_param: None,
            private_mode: false,
            bold: false,
            inverse: false,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            saved_x: 0,
            saved_y: 0,
            app_cursor_keys: false,
        }
    }

    /// Process a chunk of bytes from the PTY.
    pub fn process(&mut self, data: &[u8]) {
        for &byte in data {
            self.process_byte(byte);
        }
    }

    /// Generate a response string for device status queries.
    /// Returns bytes to send back to the PTY.
    pub fn take_response(&mut self) -> Option<Vec<u8>> {
        // Responses are generated inline during process_byte
        None
    }

    fn process_byte(&mut self, byte: u8) {
        match self.state {
            State::Normal => self.process_normal(byte),
            State::Escape => self.process_escape(byte),
            State::CSI | State::CSIParam => self.process_csi(byte),
            State::OSC => self.process_osc(byte),
        }
    }

    fn process_normal(&mut self, byte: u8) {
        match byte {
            0x1B => {
                // ESC
                self.state = State::Escape;
            }
            0x0D => {
                // CR
                self.cursor_x = 0;
            }
            0x0A | 0x0B | 0x0C => {
                // LF, VT, FF
                self.line_feed();
            }
            0x08 => {
                // BS
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            0x09 => {
                // TAB
                self.cursor_x = ((self.cursor_x / 8) + 1) * 8;
                if self.cursor_x >= self.cols {
                    self.cursor_x = self.cols - 1;
                }
            }
            0x07 => {
                // BEL - ignore on e-ink
            }
            0x00..=0x1F => {
                // Other control chars - ignore
            }
            _ => {
                // Printable character
                self.put_char(byte);
            }
        }
    }

    fn process_escape(&mut self, byte: u8) {
        match byte {
            b'[' => {
                self.state = State::CSI;
                self.params.clear();
                self.current_param = None;
                self.private_mode = false;
            }
            b']' => {
                self.state = State::OSC;
            }
            b'(' | b')' | b'*' | b'+' => {
                // Select character set - consume next byte
                self.state = State::Normal;
            }
            b'D' => {
                // Index - move down one line, scroll if needed
                self.line_feed();
                self.state = State::Normal;
            }
            b'M' => {
                // Reverse Index - move up one line, scroll if needed
                self.reverse_line_feed();
                self.state = State::Normal;
            }
            b'E' => {
                // Next Line
                self.cursor_x = 0;
                self.line_feed();
                self.state = State::Normal;
            }
            b'7' => {
                // Save cursor (DECSC)
                self.saved_x = self.cursor_x;
                self.saved_y = self.cursor_y;
                self.state = State::Normal;
            }
            b'8' => {
                // Restore cursor (DECRC)
                self.cursor_x = self.saved_x.min(self.cols - 1);
                self.cursor_y = self.saved_y.min(self.rows - 1);
                self.state = State::Normal;
            }
            b'c' => {
                // Full reset (RIS)
                self.reset();
                self.state = State::Normal;
            }
            b'>' | b'=' => {
                // Keypad modes - ignore
                self.state = State::Normal;
            }
            _ => {
                self.state = State::Normal;
            }
        }
    }

    fn process_csi(&mut self, byte: u8) {
        match byte {
            b'?' => {
                self.private_mode = true;
                self.state = State::CSIParam;
            }
            b'>' => {
                // Secondary DA prefix - ignore
                self.state = State::CSIParam;
            }
            b'0'..=b'9' => {
                let digit = (byte - b'0') as u32;
                let val = self.current_param.unwrap_or(0);
                self.current_param = Some(val * 10 + digit);
                self.state = State::CSIParam;
            }
            b';' => {
                self.params.push(self.current_param.unwrap_or(0));
                self.current_param = None;
                self.state = State::CSIParam;
            }
            b'A' => {
                // Cursor Up
                let n = self.finish_params_single(1) as usize;
                self.cursor_y = self.cursor_y.saturating_sub(n);
                self.state = State::Normal;
            }
            b'B' => {
                // Cursor Down
                let n = self.finish_params_single(1) as usize;
                self.cursor_y = (self.cursor_y + n).min(self.rows - 1);
                self.state = State::Normal;
            }
            b'C' => {
                // Cursor Forward
                let n = self.finish_params_single(1) as usize;
                self.cursor_x = (self.cursor_x + n).min(self.cols - 1);
                self.state = State::Normal;
            }
            b'D' => {
                // Cursor Backward
                let n = self.finish_params_single(1) as usize;
                self.cursor_x = self.cursor_x.saturating_sub(n);
                self.state = State::Normal;
            }
            b'E' => {
                // Cursor Next Line
                let n = self.finish_params_single(1) as usize;
                self.cursor_x = 0;
                self.cursor_y = (self.cursor_y + n).min(self.rows - 1);
                self.state = State::Normal;
            }
            b'F' => {
                // Cursor Previous Line
                let n = self.finish_params_single(1) as usize;
                self.cursor_x = 0;
                self.cursor_y = self.cursor_y.saturating_sub(n);
                self.state = State::Normal;
            }
            b'G' | b'`' => {
                // Cursor to column
                let n = self.finish_params_single(1) as usize;
                self.cursor_x = (n.saturating_sub(1)).min(self.cols - 1);
                self.state = State::Normal;
            }
            b'H' | b'f' => {
                // Cursor Position
                let (row, col) = self.finish_params_pair(1, 1);
                self.cursor_y = ((row as usize).saturating_sub(1)).min(self.rows - 1);
                self.cursor_x = ((col as usize).saturating_sub(1)).min(self.cols - 1);
                self.state = State::Normal;
            }
            b'J' => {
                // Erase in Display
                let n = self.finish_params_single(0);
                self.erase_display(n);
                self.state = State::Normal;
            }
            b'K' => {
                // Erase in Line
                let n = self.finish_params_single(0);
                self.erase_line(n);
                self.state = State::Normal;
            }
            b'L' => {
                // Insert Lines
                let n = self.finish_params_single(1) as usize;
                self.insert_lines(n);
                self.state = State::Normal;
            }
            b'M' => {
                // Delete Lines
                let n = self.finish_params_single(1) as usize;
                self.delete_lines(n);
                self.state = State::Normal;
            }
            b'P' => {
                // Delete Characters
                let n = self.finish_params_single(1) as usize;
                self.delete_chars(n);
                self.state = State::Normal;
            }
            b'@' => {
                // Insert Characters
                let n = self.finish_params_single(1) as usize;
                self.insert_chars(n);
                self.state = State::Normal;
            }
            b'S' => {
                // Scroll Up
                let n = self.finish_params_single(1) as usize;
                for _ in 0..n {
                    self.scroll_up();
                }
                self.state = State::Normal;
            }
            b'T' => {
                // Scroll Down
                let n = self.finish_params_single(1) as usize;
                for _ in 0..n {
                    self.scroll_down();
                }
                self.state = State::Normal;
            }
            b'X' => {
                // Erase Characters
                let n = self.finish_params_single(1) as usize;
                for i in 0..n {
                    let x = self.cursor_x + i;
                    if x < self.cols {
                        self.grid[self.cursor_y][x] = Cell::default();
                    }
                }
                self.dirty = true;
                self.state = State::Normal;
            }
            b'd' => {
                // Cursor to row (VPA)
                let n = self.finish_params_single(1) as usize;
                self.cursor_y = (n.saturating_sub(1)).min(self.rows - 1);
                self.state = State::Normal;
            }
            b'm' => {
                // SGR - Set Graphic Rendition
                self.process_sgr();
                self.state = State::Normal;
            }
            b'r' => {
                // Set Scrolling Region (DECSTBM)
                let (top, bottom) = self.finish_params_pair(1, self.rows as u32);
                self.scroll_top = (top as usize).saturating_sub(1).min(self.rows - 1);
                self.scroll_bottom = (bottom as usize).saturating_sub(1).min(self.rows - 1);
                if self.scroll_top >= self.scroll_bottom {
                    self.scroll_top = 0;
                    self.scroll_bottom = self.rows - 1;
                }
                self.cursor_x = 0;
                self.cursor_y = 0;
                self.state = State::Normal;
            }
            b's' => {
                // Save cursor position
                self.saved_x = self.cursor_x;
                self.saved_y = self.cursor_y;
                self.state = State::Normal;
            }
            b'u' => {
                // Restore cursor position
                self.cursor_x = self.saved_x.min(self.cols - 1);
                self.cursor_y = self.saved_y.min(self.rows - 1);
                self.state = State::Normal;
            }
            b'h' => {
                // Set Mode
                self.finish_params();
                if self.private_mode {
                    for &p in &self.params {
                        match p {
                            1 => self.app_cursor_keys = true, // DECCKM
                            25 => {} // Show cursor (we always show it)
                            _ => {}
                        }
                    }
                }
                self.state = State::Normal;
            }
            b'l' => {
                // Reset Mode
                self.finish_params();
                if self.private_mode {
                    for &p in &self.params {
                        match p {
                            1 => self.app_cursor_keys = false, // DECCKM
                            25 => {} // Hide cursor
                            _ => {}
                        }
                    }
                }
                self.state = State::Normal;
            }
            b'c' => {
                // Device Attributes - we could respond but skip for simplicity
                self.finish_params();
                self.state = State::Normal;
            }
            b'n' => {
                // Device Status Report - skip
                self.finish_params();
                self.state = State::Normal;
            }
            b't' => {
                // Window manipulation - ignore
                self.finish_params();
                self.state = State::Normal;
            }
            _ => {
                // Unknown CSI sequence - bail
                self.state = State::Normal;
            }
        }
    }

    fn process_osc(&mut self, byte: u8) {
        // OSC sequences end with BEL (0x07) or ST (ESC \)
        // We just consume and ignore them (they set window titles, etc.)
        match byte {
            0x07 => {
                self.state = State::Normal;
            }
            0x1B => {
                // Might be ST (ESC \) - go to escape state briefly
                self.state = State::Normal;
            }
            _ => {
                // Continue consuming OSC
            }
        }
    }

    fn finish_params(&mut self) {
        if let Some(p) = self.current_param {
            self.params.push(p);
        }
        self.current_param = None;
    }

    fn finish_params_single(&mut self, default: u32) -> u32 {
        self.finish_params();
        if self.params.is_empty() {
            default
        } else {
            let v = self.params[0];
            if v == 0 { default } else { v }
        }
    }

    fn finish_params_pair(&mut self, def1: u32, def2: u32) -> (u32, u32) {
        self.finish_params();
        let a = if self.params.len() > 0 && self.params[0] != 0 {
            self.params[0]
        } else {
            def1
        };
        let b = if self.params.len() > 1 && self.params[1] != 0 {
            self.params[1]
        } else {
            def2
        };
        (a, b)
    }

    fn process_sgr(&mut self) {
        self.finish_params();
        if self.params.is_empty() {
            self.params.push(0);
        }
        let mut i = 0;
        while i < self.params.len() {
            match self.params[i] {
                0 => {
                    // Reset
                    self.bold = false;
                    self.inverse = false;
                }
                1 => self.bold = true,
                7 => self.inverse = true,
                22 => self.bold = false,
                27 => self.inverse = false,
                // Colors: we just ignore them for e-ink (no color)
                // but we accept them so the parser doesn't break
                30..=37 | 39 | 40..=47 | 49 | 90..=97 | 100..=107 => {}
                38 | 48 => {
                    // Extended color: 38;5;N or 38;2;R;G;B
                    if i + 1 < self.params.len() {
                        match self.params[i + 1] {
                            5 => {
                                i += 2; // Skip ;5;N
                            }
                            2 => {
                                i += 4; // Skip ;2;R;G;B
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn put_char(&mut self, ch: u8) {
        if self.cursor_x >= self.cols {
            // Line wrap
            self.cursor_x = 0;
            self.line_feed();
        }
        self.grid[self.cursor_y][self.cursor_x] = Cell {
            ch,
            bold: self.bold,
            inverse: self.inverse,
            dirty: true,
        };
        self.cursor_x += 1;
        self.dirty = true;
    }

    fn line_feed(&mut self) {
        if self.cursor_y == self.scroll_bottom {
            self.scroll_up();
        } else if self.cursor_y < self.rows - 1 {
            self.cursor_y += 1;
        }
    }

    fn reverse_line_feed(&mut self) {
        if self.cursor_y == self.scroll_top {
            self.scroll_down();
        } else if self.cursor_y > 0 {
            self.cursor_y -= 1;
        }
    }

    fn scroll_up(&mut self) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        // Remove top line, insert blank at bottom
        self.grid.remove(top);
        self.grid.insert(bottom, vec![Cell::default(); self.cols]);
        // Mark all rows in scroll region dirty
        for row in top..=bottom {
            for cell in &mut self.grid[row] {
                cell.dirty = true;
            }
        }
        self.dirty = true;
    }

    fn scroll_down(&mut self) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        // Remove bottom line, insert blank at top
        self.grid.remove(bottom);
        self.grid.insert(top, vec![Cell::default(); self.cols]);
        for row in top..=bottom {
            for cell in &mut self.grid[row] {
                cell.dirty = true;
            }
        }
        self.dirty = true;
    }

    fn erase_display(&mut self, mode: u32) {
        match mode {
            0 => {
                // Erase below (from cursor to end)
                self.erase_line(0);
                for row in (self.cursor_y + 1)..self.rows {
                    for col in 0..self.cols {
                        self.grid[row][col] = Cell::default();
                    }
                }
            }
            1 => {
                // Erase above (from start to cursor)
                for row in 0..self.cursor_y {
                    for col in 0..self.cols {
                        self.grid[row][col] = Cell::default();
                    }
                }
                self.erase_line(1);
            }
            2 | 3 => {
                // Erase all
                for row in 0..self.rows {
                    for col in 0..self.cols {
                        self.grid[row][col] = Cell::default();
                    }
                }
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn erase_line(&mut self, mode: u32) {
        let row = self.cursor_y;
        match mode {
            0 => {
                // Erase right (from cursor to end of line)
                for col in self.cursor_x..self.cols {
                    self.grid[row][col] = Cell::default();
                }
            }
            1 => {
                // Erase left (from start to cursor)
                for col in 0..=self.cursor_x.min(self.cols - 1) {
                    self.grid[row][col] = Cell::default();
                }
            }
            2 => {
                // Erase whole line
                for col in 0..self.cols {
                    self.grid[row][col] = Cell::default();
                }
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn insert_lines(&mut self, n: usize) {
        let top = self.cursor_y;
        let bottom = self.scroll_bottom;
        for _ in 0..n.min(bottom - top + 1) {
            self.grid.remove(bottom);
            self.grid.insert(top, vec![Cell::default(); self.cols]);
        }
        for row in top..=bottom {
            for cell in &mut self.grid[row] {
                cell.dirty = true;
            }
        }
        self.dirty = true;
    }

    fn delete_lines(&mut self, n: usize) {
        let top = self.cursor_y;
        let bottom = self.scroll_bottom;
        for _ in 0..n.min(bottom - top + 1) {
            self.grid.remove(top);
            self.grid.insert(bottom, vec![Cell::default(); self.cols]);
        }
        for row in top..=bottom {
            for cell in &mut self.grid[row] {
                cell.dirty = true;
            }
        }
        self.dirty = true;
    }

    fn delete_chars(&mut self, n: usize) {
        let row = self.cursor_y;
        let x = self.cursor_x;
        for _ in 0..n {
            if x < self.cols {
                self.grid[row].remove(x);
                self.grid[row].push(Cell::default());
            }
        }
        for cell in &mut self.grid[row][x..] {
            cell.dirty = true;
        }
        self.dirty = true;
    }

    fn insert_chars(&mut self, n: usize) {
        let row = self.cursor_y;
        let x = self.cursor_x;
        for _ in 0..n {
            if self.grid[row].len() > 0 {
                self.grid[row].pop();
                self.grid[row].insert(x, Cell::default());
            }
        }
        for cell in &mut self.grid[row][x..] {
            cell.dirty = true;
        }
        self.dirty = true;
    }

    fn reset(&mut self) {
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.bold = false;
        self.inverse = false;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        self.app_cursor_keys = false;
        self.erase_display(2);
    }

    /// Mark all cells as clean (call after rendering).
    pub fn mark_clean(&mut self) {
        for row in &mut self.grid {
            for cell in row {
                cell.dirty = false;
            }
        }
        self.dirty = false;
    }
}
