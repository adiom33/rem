/// Boot-time launcher menu for choosing between Terminal and Reader modes.
///
/// Draws a simple menu on the framebuffer with two tappable buttons.
/// Works alongside xochitl via rm2fb — the menu is drawn on top of the
/// reader UI in shared memory.

use crate::font;
use crate::framebuffer::Framebuffer;
use crate::input::Input;
use crate::sys;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LauncherChoice {
    Terminal,
    Reader,
}

const MENU_SCALE: usize = 3;
const TITLE_SCALE: usize = 4;
const BORDER_PX: usize = 3;
const BTN_CHARS_W: usize = 18;
const BTN_CHARS_H: usize = 3;

pub fn run(fb: &mut Framebuffer) -> LauncherChoice {
    let char_w = font::FONT_WIDTH * MENU_SCALE;
    let char_h = font::FONT_HEIGHT * MENU_SCALE;

    let screen_w = fb.width;
    let screen_h = fb.height;

    // Button dimensions
    let btn_w = BTN_CHARS_W * char_w;
    let btn_h = BTN_CHARS_H * char_h;
    let btn_x = (screen_w - btn_w) / 2;

    // Vertical layout
    let title_y = screen_h / 5;
    let title_char_w = font::FONT_WIDTH * TITLE_SCALE;
    let title_char_h = font::FONT_HEIGHT * TITLE_SCALE;
    let subtitle_y = title_y + title_char_h + char_h / 2;
    let btn1_y = screen_h * 2 / 5;
    let btn2_y = btn1_y + btn_h + char_h;

    // Draw screen
    fb.clear();

    // Title
    let title = "rem";
    let title_x = (screen_w - title.len() * title_char_w) / 2;
    fb.draw_str(title, title_x, title_y, TITLE_SCALE, false);

    // Subtitle
    let subtitle = "remarkable terminal";
    let sub_x = (screen_w - subtitle.len() * char_w) / 2;
    fb.draw_str(subtitle, sub_x, subtitle_y, MENU_SCALE, false);

    // Buttons
    let labels = ["Terminal", "Reader"];
    let btn_ys = [btn1_y, btn2_y];

    for (i, &label) in labels.iter().enumerate() {
        draw_button(fb, label, btn_x, btn_ys[i], btn_w, btn_h);
    }

    fb.refresh_full();

    // Wait for touch
    let mut input = match Input::open(screen_w as i32, screen_h as i32) {
        Ok(inp) => inp,
        Err(e) => {
            eprintln!("Launcher: failed to open input: {}", e);
            return LauncherChoice::Terminal;
        }
    };

    loop {
        let fds = input.fds();
        let mut poll_fds: Vec<sys::pollfd> = fds
            .iter()
            .map(|&fd| sys::pollfd {
                fd,
                events: sys::POLLIN,
                revents: 0,
            })
            .collect();

        let ret = unsafe {
            sys::poll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as sys::nfds_t,
                1000,
            )
        };

        if ret <= 0 {
            continue;
        }

        for i in 0..poll_fds.len() {
            if poll_fds[i].revents & sys::POLLIN == 0 {
                continue;
            }

            let events = input.read_events(i);
            for ev in events {
                if ev.pressure <= 0 {
                    continue;
                }

                // Check which button was tapped
                for (idx, &by) in btn_ys.iter().enumerate() {
                    if ev.x >= btn_x as i32
                        && ev.x < (btn_x + btn_w) as i32
                        && ev.y >= by as i32
                        && ev.y < (by + btn_h) as i32
                    {
                        // Visual feedback: invert the button
                        let label = labels[idx];
                        fb.fill_rect(
                            btn_x + BORDER_PX,
                            by + BORDER_PX,
                            btn_w - 2 * BORDER_PX,
                            btn_h - 2 * BORDER_PX,
                            false,
                        );
                        let label_x = btn_x + (btn_w - label.len() * char_w) / 2;
                        let label_y = by + (btn_h - char_h) / 2;
                        fb.draw_str(label, label_x, label_y, MENU_SCALE, true);
                        fb.refresh_full();
                        std::thread::sleep(std::time::Duration::from_millis(250));

                        // Clear screen before transitioning
                        fb.clear();
                        fb.refresh_full();

                        return match idx {
                            0 => LauncherChoice::Terminal,
                            _ => LauncherChoice::Reader,
                        };
                    }
                }
            }
        }
    }
}

fn draw_button(fb: &mut Framebuffer, label: &str, x: usize, y: usize, w: usize, h: usize) {
    let char_w = font::FONT_WIDTH * MENU_SCALE;
    let char_h = font::FONT_HEIGHT * MENU_SCALE;

    // Border
    for t in 0..BORDER_PX {
        for col in x..x + w {
            fb.set_pixel(col, y + t, false);
            fb.set_pixel(col, y + h - 1 - t, false);
        }
        for row in y..y + h {
            fb.set_pixel(x + t, row, false);
            fb.set_pixel(x + w - 1 - t, row, false);
        }
    }

    // Centered label
    let label_x = x + (w - label.len() * char_w) / 2;
    let label_y = y + (h - char_h) / 2;
    fb.draw_str(label, label_x, label_y, MENU_SCALE, false);
}
