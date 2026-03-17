mod font;
mod framebuffer;
mod input;
mod keyboard;
mod launcher;
mod pty;
mod setup;
#[allow(non_camel_case_types, dead_code)]
mod sys;
mod terminal;

use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use framebuffer::Framebuffer;
use input::Input;
use keyboard::{OnScreenKeyboard, PhysicalKeyboard};
use pty::Pty;
use terminal::Terminal;

/// Default font scale. 2x means 16x32 pixel characters.
/// On the rM2's 1404x1872 display this gives ~87 columns x 58 rows (full screen)
/// or ~87 cols x 37 rows with on-screen keyboard.
const DEFAULT_FONT_SCALE: usize = 3;

/// How many milliseconds to wait before refreshing the display after PTY output.
/// Allows batching rapid output (e.g. scrolling) into a single refresh.
const REFRESH_DEBOUNCE_MS: u64 = 50;

/// Status bar height in font-rows
const STATUS_BAR_ROWS: usize = 1;

fn print_usage() {
    eprintln!("remarkable-ssh - framebuffer terminal for reMarkable 2");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("  remarkable-ssh [OPTIONS] [user@host]");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("  --scale N       Font scale factor (default: 2)");
    eprintln!("  --fb PATH       Framebuffer device (default: /dev/fb0)");
    eprintln!("  --shell CMD     Shell to run if no SSH target given (default: /bin/sh)");
    eprintln!("  --keyboard      Enable on-screen virtual keyboard (auto-enabled if no physical keyboard)");
    eprintln!("  --no-keyboard   Disable on-screen keyboard (even if no physical keyboard found)");
    eprintln!("  --kb PATH       Keyboard device path (e.g. /dev/input/event3)");
    eprintln!("  --ssh-cmd CMD   SSH client binary (default: ssh, fallback: dbclient)");
    eprintln!("  --tmux          Auto-attach/create tmux session on remote host");
    eprintln!("  --cmd STRING    Remote command to run via SSH");
    eprintln!("  --ssh-args ARGS Extra arguments passed to SSH (comma-separated)");
    eprintln!("  --ssh-arg ARG   Extra SSH argument (repeatable)");
    eprintln!("  --landscape     Force landscape (90° CW rotation for folio keyboard)");
    eprintln!("  --portrait      Force portrait mode (no rotation)");
    eprintln!("  --launcher      Show boot menu (Terminal / Reader chooser)");
    eprintln!("  --setup         Auto-detect rm2fb addresses and write /etc/rm2fb.conf");
    eprintln!("  --help          Show this help");
    eprintln!();
    eprintln!("EXAMPLES:");
    eprintln!("  remarkable-ssh user@192.168.1.100");
    eprintln!("  remarkable-ssh user@100.64.0.1       # Tailscale IP");
    eprintln!("  remarkable-ssh --shell /bin/bash      # Local shell");
    eprintln!("  remarkable-ssh --keyboard user@myhost # With on-screen keyboard");
    eprintln!("  remarkable-ssh --tmux user@myhost     # Auto-attach to tmux");
    eprintln!("  remarkable-ssh --cmd htop user@myhost # Run htop remotely");
    eprintln!("  remarkable-ssh --kb /dev/input/event3 user@myhost");
    eprintln!("  remarkable-ssh --ssh-arg -i --ssh-arg /path/to/key user@host");
}

struct Config {
    font_scale: usize,
    fb_path: String,
    shell: String,
    ssh_cmd: String,
    ssh_target: Option<String>,
    show_keyboard: bool,
    no_keyboard: bool,
    kb_path: Option<String>,
    tmux: bool,
    remote_cmd: Option<String>,
    ssh_extra_args: Vec<String>,
    launcher: bool,
    hotkey: bool,
    landscape: Option<bool>, // None=auto, Some(true)=force landscape, Some(false)=force portrait
    allow_degraded: bool,
}

fn parse_args() -> Config {
    let args: Vec<String> = env::args().collect();
    let mut config = Config {
        font_scale: DEFAULT_FONT_SCALE,
        fb_path: "/dev/fb0".to_string(),
        shell: "/bin/sh".to_string(),
        ssh_cmd: "ssh".to_string(),
        ssh_target: None,
        show_keyboard: false,
        no_keyboard: false,
        kb_path: None,
        tmux: false,
        remote_cmd: None,
        ssh_extra_args: Vec::new(),
        launcher: false,
        hotkey: false,
        landscape: None,
        allow_degraded: false,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--scale" => {
                i += 1;
                if i < args.len() {
                    config.font_scale = args[i].parse().unwrap_or(DEFAULT_FONT_SCALE).max(1);
                }
            }
            "--fb" => {
                i += 1;
                if i < args.len() {
                    config.fb_path = args[i].clone();
                }
            }
            "--shell" => {
                i += 1;
                if i < args.len() {
                    config.shell = args[i].clone();
                }
            }
            "--ssh-cmd" => {
                i += 1;
                if i < args.len() {
                    config.ssh_cmd = args[i].clone();
                }
            }
            "--keyboard" => {
                config.show_keyboard = true;
            }
            "--no-keyboard" => {
                config.no_keyboard = true;
            }
            "--kb" => {
                i += 1;
                if i < args.len() {
                    config.kb_path = Some(args[i].clone());
                }
            }
            "--tmux" => {
                config.tmux = true;
            }
            "--cmd" => {
                i += 1;
                if i < args.len() {
                    config.remote_cmd = Some(args[i].clone());
                }
            }
            "--ssh-args" => {
                i += 1;
                if i < args.len() {
                    for arg in args[i].split(',') {
                        let trimmed = arg.trim();
                        if !trimmed.is_empty() {
                            config.ssh_extra_args.push(trimmed.to_string());
                        }
                    }
                }
            }
            "--ssh-arg" => {
                i += 1;
                if i < args.len() {
                    config.ssh_extra_args.push(args[i].clone());
                }
            }
            "--landscape" => {
                config.landscape = Some(true);
            }
            "--portrait" => {
                config.landscape = Some(false);
            }
            "--launcher" => {
                config.launcher = true;
            }
            "--hotkey" => {
                config.hotkey = true;
            }
            "--allow-degraded" => {
                config.allow_degraded = true;
            }
            "--setup" => {
                // Run rm2fb auto-setup and exit
                match setup::run_setup() {
                    Ok(()) => std::process::exit(0),
                    Err(e) => {
                        eprintln!("Setup failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            arg => {
                if arg.starts_with('-') {
                    eprintln!("Unknown option: {}", arg);
                    print_usage();
                    std::process::exit(1);
                }
                config.ssh_target = Some(arg.to_string());
            }
        }
        i += 1;
    }

    config
}

fn find_ssh_cmd(preferred: &str) -> String {
    if command_exists(preferred) {
        return preferred.to_string();
    }
    for cmd in &["ssh", "dbclient", "/usr/bin/ssh", "/usr/bin/dbclient"] {
        if command_exists(cmd) {
            eprintln!("Using SSH client: {}", cmd);
            return cmd.to_string();
        }
    }
    "ssh".to_string()
}

fn command_exists(cmd: &str) -> bool {
    if cmd.starts_with('/') {
        std::path::Path::new(cmd).exists()
    } else {
        if let Ok(path) = env::var("PATH") {
            for dir in path.split(':') {
                if std::path::Path::new(dir).join(cmd).exists() {
                    return true;
                }
            }
        }
        false
    }
}

fn render_terminal(
    fb: &mut Framebuffer,
    term: &Terminal,
    scale: usize,
) {
    let char_w = font::FONT_WIDTH * scale;
    let char_h = font::FONT_HEIGHT * scale;

    for row in 0..term.rows {
        for col in 0..term.cols {
            let cell = &term.grid[row][col];
            if !cell.dirty {
                continue;
            }
            let px = col * char_w;
            let py = row * char_h;

            let at_cursor = term.cursor_visible && row == term.cursor_y && col == term.cursor_x;
            let inverse = cell.inverse ^ at_cursor;

            fb.draw_char(cell.ch, px, py, scale, inverse, cell.bold);
        }
    }
}

fn render_status_bar(
    fb: &mut Framebuffer,
    term: &Terminal,
    scale: usize,
    y: usize,
    target: &str,
) {
    let char_h = font::FONT_HEIGHT * scale;
    let bar_w = fb.width;

    fb.fill_rect(0, y, bar_w, char_h, false);

    let status = format!(
        " {} | {}x{} | Ln {} Col {} ",
        target,
        term.cols,
        term.rows,
        term.cursor_y + 1,
        term.cursor_x + 1,
    );
    fb.draw_str(&status, 0, y, scale, true);
}

/// Global flag: set to true when we've stopped xochitl and need to restart it on exit.
static XOCHITL_STOPPED: AtomicBool = AtomicBool::new(false);

/// Global flag: set by signal handler to request clean exit from main loop.
static SIGNAL_EXIT: AtomicBool = AtomicBool::new(false);

/// Restart xochitl using systemctl. NOT safe to call from signal handlers.
fn restart_xochitl() {
    if XOCHITL_STOPPED.swap(false, Ordering::SeqCst) {
        eprintln!("Restarting xochitl...");
        let cmd = b"systemctl start xochitl\0";
        unsafe {
            sys::system(cmd.as_ptr() as *const sys::c_char);
        }
    }
}

/// Signal handler for SIGINT/SIGTERM — sets exit flag only (async-signal-safe).
/// The main loop checks this flag and does the actual cleanup.
extern "C" fn signal_handler(_sig: sys::c_int) {
    SIGNAL_EXIT.store(true, Ordering::SeqCst);
}

/// Stop xochitl so we can own the framebuffer. Installs safety hooks to restart it.
fn stop_xochitl() {
    // Check if xochitl is running
    let cmd = b"systemctl is-active --quiet xochitl\0";
    let ret = unsafe { sys::system(cmd.as_ptr() as *const sys::c_char) };
    if ret != 0 {
        eprintln!("xochitl is not running, skipping stop.");
        return;
    }

    eprintln!("Stopping xochitl...");
    let stop_cmd = b"systemctl stop xochitl\0";
    unsafe {
        sys::system(stop_cmd.as_ptr() as *const sys::c_char);
    }
    XOCHITL_STOPPED.store(true, Ordering::SeqCst);

    // Install signal handlers
    unsafe {
        sys::signal(sys::SIGINT, signal_handler);
        sys::signal(sys::SIGTERM, signal_handler);
    }

    // Install panic hook to restart xochitl
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restart_xochitl();
        default_hook(info);
    }));
}

fn run_hotkey_mode() {
    use std::fs::{File, OpenOptions};
    use std::io::Read;
    use std::os::unix::io::AsRawFd;

    const EV_SYN: u16 = 0;
    const EV_KEY: u16 = 1;
    const EV_REP: u16 = 0x14;
    const KEY_LEFTCTRL: u16 = 29;
    const KEY_LEFTALT: u16 = 56;
    const KEY_T: u16 = 20;

    // EVIOCGRAB = _IOW('E', 0x90, int) = 0x40044590
    const EVIOCGRAB: sys::c_ulong = 0x40044590;

    // uinput ioctls
    const UI_DEV_SETUP: sys::c_ulong = 0x405c5503;  // _IOW('U', 3, uinput_setup)
    const UI_DEV_CREATE: sys::c_ulong = 0x5501;      // _IO('U', 1)
    const UI_SET_EVBIT: sys::c_ulong = 0x40045564;   // _IOW('U', 100, int)
    const UI_SET_KEYBIT: sys::c_ulong = 0x40045565;  // _IOW('U', 101, int)

    // Find the keyboard
    let mut kb_fd = -1i32;
    let mut kb_file: Option<File> = None;
    for i in 0..20 {
        let path = format!("/dev/input/event{}", i);
        if let Ok(f) = File::open(&path) {
            let fd = f.as_raw_fd();
            let mut name_buf = [0u8; 256];
            let ioctl_num: sys::c_ulong = (2 << 30) | (256 << 16) | (0x45 << 8) | 0x06;
            let ret = unsafe { sys::ioctl(fd, ioctl_num, name_buf.as_mut_ptr()) };
            if ret > 0 {
                let name = String::from_utf8_lossy(&name_buf[..ret as usize]);
                if name.to_lowercase().contains("keyboard") {
                    eprintln!("Hotkey daemon: found keyboard at {} ({})", path, name.trim_end_matches('\0'));
                    kb_fd = fd;
                    kb_file = Some(f);
                    break;
                }
            }
        }
    }

    let mut kb_file = match kb_file {
        Some(f) => f,
        None => {
            eprintln!("Hotkey daemon: no keyboard found, falling back to trigger file mode");
            run_trigger_mode();
            return;
        }
    };

    // Grab the keyboard exclusively so xochitl can't use it
    let grab: sys::c_int = 1;
    let ret = unsafe { sys::ioctl(kb_fd, EVIOCGRAB, &grab) };
    if ret < 0 {
        eprintln!("Hotkey daemon: EVIOCGRAB failed (errno {}), falling back to trigger mode", sys::errno());
        run_trigger_mode();
        return;
    }
    eprintln!("Hotkey daemon: grabbed keyboard exclusively");

    // Create a virtual keyboard via uinput to forward events to xochitl
    let uinput_file = match OpenOptions::new().write(true).open("/dev/uinput") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Hotkey daemon: can't open /dev/uinput: {}, releasing grab", e);
            let ungrab: sys::c_int = 0;
            unsafe { sys::ioctl(kb_fd, EVIOCGRAB, &ungrab); }
            run_trigger_mode();
            return;
        }
    };
    let ui_fd = uinput_file.as_raw_fd();

    // Set up the virtual keyboard capabilities
    unsafe {
        // Enable EV_SYN, EV_KEY, EV_REP
        sys::ioctl(ui_fd, UI_SET_EVBIT, EV_SYN as sys::c_int);
        sys::ioctl(ui_fd, UI_SET_EVBIT, EV_KEY as sys::c_int);
        sys::ioctl(ui_fd, UI_SET_EVBIT, EV_REP as sys::c_int);

        // Enable all key codes 0-255
        for key in 0..256 {
            sys::ioctl(ui_fd, UI_SET_KEYBIT, key as sys::c_int);
        }
    }

    // uinput_setup struct: u16 bustype, u16 vendor, u16 product, u16 version, char[80] name, u32 ff_effects_max
    // Total = 2+2+2+2+80+4 = 92 bytes
    let mut setup = [0u8; 92];
    // bus=0x19, vendor=0x2edd, product=0x0001, version=0x0100 (match real keyboard)
    setup[0..2].copy_from_slice(&0x0019u16.to_ne_bytes());
    setup[2..4].copy_from_slice(&0x2eddu16.to_ne_bytes());
    setup[4..6].copy_from_slice(&0x0001u16.to_ne_bytes());
    setup[6..8].copy_from_slice(&0x0100u16.to_ne_bytes());
    let name = b"rM_Keyboard_Proxy";
    setup[8..8 + name.len()].copy_from_slice(name);

    let ret = unsafe { sys::ioctl(ui_fd, UI_DEV_SETUP, setup.as_ptr()) };
    if ret < 0 {
        eprintln!("Hotkey daemon: UI_DEV_SETUP failed (errno {})", sys::errno());
    }

    let ret = unsafe { sys::ioctl(ui_fd, UI_DEV_CREATE, 0 as sys::c_int) };
    if ret < 0 {
        eprintln!("Hotkey daemon: UI_DEV_CREATE failed (errno {})", sys::errno());
        let ungrab: sys::c_int = 0;
        unsafe { sys::ioctl(kb_fd, EVIOCGRAB, &ungrab); }
        return;
    }

    eprintln!("Hotkey daemon: virtual keyboard created, forwarding events (Ctrl+Alt+T to launch terminal)");

    // Give xochitl time to discover the new virtual keyboard
    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut ctrl = false;
    let mut alt = false;
    let mut buf = [0u8; 16];

    loop {
        if kb_file.read_exact(&mut buf).is_err() {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }

        let ev_type = u16::from_ne_bytes([buf[8], buf[9]]);
        let ev_code = u16::from_ne_bytes([buf[10], buf[11]]);
        let ev_value = i32::from_ne_bytes([buf[12], buf[13], buf[14], buf[15]]);

        // Track modifier state
        if ev_type == EV_KEY {
            match ev_code {
                KEY_LEFTCTRL => ctrl = ev_value != 0,
                KEY_LEFTALT => alt = ev_value != 0,
                KEY_T if ev_value == 1 && ctrl && alt => {
                    eprintln!("Hotkey daemon: Ctrl+Alt+T — launching terminal");

                    // Stop forwarding to xochitl while terminal is active
                    let exe = std::env::current_exe()
                        .unwrap_or_else(|_| std::path::PathBuf::from("/opt/bin/remarkable-ssh"));
                    // Release grab so terminal can use the keyboard directly
                    let ungrab: sys::c_int = 0;
                    unsafe { sys::ioctl(kb_fd, EVIOCGRAB, &ungrab); }

                    match std::process::Command::new(&exe).status() {
                        Ok(s) => eprintln!("Hotkey daemon: terminal exited ({})", s),
                        Err(e) => eprintln!("Hotkey daemon: failed to launch: {}", e),
                    }

                    // Re-grab keyboard
                    let grab: sys::c_int = 1;
                    unsafe { sys::ioctl(kb_fd, EVIOCGRAB, &grab); }

                    ctrl = false;
                    alt = false;
                    continue;
                }
                _ => {}
            }
        }

        // Forward the event to the virtual keyboard (for xochitl)
        unsafe {
            sys::write(ui_fd, buf.as_ptr() as *const sys::c_void, 16);
        }
    }
}

/// Fallback: watch for /tmp/rem-terminal trigger file
fn run_trigger_mode() {
    eprintln!("Trigger mode: create /tmp/rem-terminal to launch terminal");
    loop {
        if std::path::Path::new("/tmp/rem-terminal").exists() {
            let _ = std::fs::remove_file("/tmp/rem-terminal");
            eprintln!("Trigger: launching terminal");
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("/opt/bin/remarkable-ssh"));
            match std::process::Command::new(&exe).status() {
                Ok(s) => eprintln!("Trigger: terminal exited ({})", s),
                Err(e) => eprintln!("Trigger: failed to launch: {}", e),
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn run_launcher_mode(config: &Config) {
    // Collect args to forward to the terminal subprocess (everything except --launcher)
    let forward_args: Vec<String> = env::args()
        .skip(1)
        .filter(|a| a != "--launcher")
        .collect();

    let exe = env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("/home/root/remarkable-ssh"));

    loop {
        let mut fb = match Framebuffer::open(&config.fb_path, config.allow_degraded) {
            Ok(fb) => fb,
            Err(e) => {
                eprintln!("Launcher: {}", e);
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
        };

        eprintln!("Launcher: showing menu (backend: {})", fb.backend_name());

        match launcher::run(&mut fb) {
            launcher::LauncherChoice::Terminal => {
                drop(fb);
                eprintln!("Launcher: starting terminal...");
                match std::process::Command::new(&exe).args(&forward_args).status() {
                    Ok(status) => {
                        eprintln!("Launcher: terminal exited with {}", status);
                    }
                    Err(e) => {
                        eprintln!("Launcher: failed to start terminal: {}", e);
                        std::thread::sleep(std::time::Duration::from_secs(3));
                    }
                }
                // Brief pause to let e-ink settle before re-drawing
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            launcher::LauncherChoice::Reader => {
                fb.clear();
                fb.refresh_full();
                drop(fb);
                eprintln!("Launcher: switching to reader (xochitl).");
                // Start xochitl and wait for it to exit.
                // When the user wants to return, they can:
                //   - Long-press power button (triggers shutdown/reboot → launcher restarts)
                //   - SSH in and run: systemctl stop xochitl
                let cmd = b"systemctl start xochitl 2>/dev/null; sleep 2; while pidof xochitl >/dev/null 2>&1; do sleep 2; done\0";
                unsafe {
                    sys::system(cmd.as_ptr() as *const sys::c_char);
                }
                eprintln!("Launcher: xochitl exited, returning to menu.");
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
}

fn main() {
    let config = parse_args();

    // ---- Launcher mode ----
    if config.launcher {
        run_launcher_mode(&config);
        return;
    }

    // ---- Hotkey daemon mode ----
    if config.hotkey {
        run_hotkey_mode();
        return;
    }

    // ---- Open framebuffer ----
    let mut fb = match Framebuffer::open(&config.fb_path, config.allow_degraded) {
        Ok(fb) => fb,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            eprintln!("Make sure you are running on a reMarkable tablet,");
            eprintln!("or specify a framebuffer device with --fb.");
            std::process::exit(1);
        }
    };

    eprintln!("Display backend: {}", fb.backend_name());

    // ---- Set landscape mode if requested or auto-detected ----
    let use_landscape = match config.landscape {
        Some(v) => v,
        None => {
            // Auto-detect: check if folio keyboard (rM_Keyboard) is present
            std::path::Path::new("/dev/input/by-path/platform-30a80000.serial-event").exists()
        }
    };
    if use_landscape {
        fb.set_landscape();
    }

    // ---- Stop xochitl so we own the framebuffer ----
    // Skip if using rm2fb — the rm2fb server runs inside xochitl (or alongside it),
    // so stopping xochitl would kill the display backend.
    if fb.is_rm2fb() {
        eprintln!("Using rm2fb backend — not stopping xochitl.");
    } else {
        stop_xochitl();
    }

    let scale = config.font_scale;
    let char_w = font::FONT_WIDTH * scale;
    let char_h = font::FONT_HEIGHT * scale;

    // ---- Open physical keyboard ----
    let mut phys_kb = if let Some(ref path) = config.kb_path {
        PhysicalKeyboard::open_path(path)
    } else {
        PhysicalKeyboard::open()
    };

    let has_physical_kb = phys_kb.fd().is_some();

    // ---- Decide whether to show on-screen keyboard ----
    // --no-keyboard: explicitly disable (e.g., display-only use)
    // --keyboard: explicitly enable (even alongside physical keyboard)
    // Default: auto-enable if no physical keyboard detected
    let show_keyboard = if config.no_keyboard {
        false
    } else if config.show_keyboard {
        true
    } else if !has_physical_kb {
        eprintln!("No physical keyboard detected — enabling on-screen keyboard automatically.");
        eprintln!("  Use --no-keyboard to disable.");
        true
    } else {
        false
    };

    // ---- Calculate terminal dimensions ----
    let kb_height = if show_keyboard {
        (fb.height / 3).max(char_h * 5)
    } else {
        0
    };

    let status_bar_height = char_h * STATUS_BAR_ROWS;
    let term_area_height = fb.height.saturating_sub(kb_height).saturating_sub(status_bar_height);
    let term_cols = fb.width / char_w;
    let term_rows = term_area_height / char_h;

    if term_cols == 0 || term_rows == 0 {
        eprintln!(
            "ERROR: Terminal dimensions are {}x{} — display too small or scale too large.",
            term_cols, term_rows,
        );
        restart_xochitl();
        std::process::exit(1);
    }

    eprintln!(
        "Terminal: {}x{} chars (scale {}x, {}x{} pixels per char)",
        term_cols, term_rows, scale, char_w, char_h,
    );

    // ---- Create terminal emulator ----
    let mut term = Terminal::new(term_cols, term_rows);

    // ---- Create on-screen keyboard ----
    let mut osk = if show_keyboard {
        Some(OnScreenKeyboard::new(
            0,
            fb.height - kb_height,
            fb.width,
            kb_height,
        ))
    } else {
        None
    };

    // ---- Open touch input (only needed for on-screen keyboard) ----
    let mut touch_input = if show_keyboard {
        Input::open(fb.width as i32, fb.height as i32, fb.rotated).ok()
    } else {
        None
    };

    // ---- Determine command to run ----
    let (command, args): (String, Vec<String>) = if let Some(ref target) = config.ssh_target {
        let ssh = find_ssh_cmd(&config.ssh_cmd);
        let mut argv = vec![ssh.clone()];

        // Force PTY allocation for tmux or remote commands
        if config.tmux || config.remote_cmd.is_some() {
            argv.push("-tt".to_string());
        }

        // Extra SSH arguments
        for arg in &config.ssh_extra_args {
            argv.push(arg.clone());
        }

        // Target host
        argv.push(target.clone());

        // Remote command: --tmux takes precedence, then --cmd
        if config.tmux {
            argv.push("tmux attach || tmux new -s main".to_string());
        } else if let Some(ref cmd) = config.remote_cmd {
            argv.push(cmd.clone());
        }

        (ssh, argv)
    } else {
        (config.shell.clone(), vec![config.shell.clone()])
    };

    let display_target = config
        .ssh_target
        .as_deref()
        .unwrap_or("local shell");

    // ---- Spawn PTY with child process ----
    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let pty = match Pty::spawn(term_cols as u16, term_rows as u16, &command, &args_refs, "xterm") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ERROR: Failed to spawn {}: {}", command, e);
            eprintln!("If SSH is not found, try: --ssh-cmd /path/to/ssh");
            restart_xochitl();
            std::process::exit(1);
        }
    };

    // ---- Warp splash screen ----
    {
        let screen_w = fb.width;
        let screen_h = fb.height;
        let cx = screen_w / 2;
        let cy = screen_h / 2;
        let max_r = screen_w.max(screen_h) * 3 / 4;

        // Simple LCG PRNG (no dependencies needed)
        let mut rng_state: u32 = 0xDEAD_BEEFu32;
        let rng_next = |state: &mut u32| -> u32 {
            *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *state
        };

        // Pre-compute sin/cos table (256 angles covering 0..2*PI)
        let mut sin_table = [0i32; 256];
        let mut cos_table = [0i32; 256];
        for i in 0..256 {
            let angle = (i as f32) * std::f32::consts::TAU / 256.0;
            sin_table[i] = (angle.sin() * 1024.0) as i32; // fixed-point 10.10
            cos_table[i] = (angle.cos() * 1024.0) as i32;
        }

        struct Star {
            angle_idx: usize, // index into sin/cos tables
            radius: i32,      // distance from center (pixels)
            speed: i32,        // pixels per frame
        }

        let num_stars = 80;
        let mut stars: Vec<Star> = (0..num_stars)
            .map(|_| Star {
                angle_idx: (rng_next(&mut rng_state) % 256) as usize,
                radius: (rng_next(&mut rng_state) % max_r as u32) as i32,
                speed: 3 + (rng_next(&mut rng_state) % 12) as i32,
            })
            .collect();

        // Draw initial frame: black background with title
        fb.fill_black();

        // Title "rem" centered — white on black
        let title_scale = 8;
        let title_cw = font::FONT_WIDTH * title_scale;
        let title_ch = font::FONT_HEIGHT * title_scale;
        let title = "rem";
        let title_x = (screen_w - title.len() * title_cw) / 2;
        let title_y = (screen_h - title_ch) / 2 - title_ch / 2;
        fb.draw_str(title, title_x, title_y, title_scale, true); // inverse = white on black

        // Subtitle below title
        let sub_cw = font::FONT_WIDTH * scale;
        let subtitle = "remarkable terminal";
        let sub_x = (screen_w - subtitle.len() * sub_cw) / 2;
        let sub_y = title_y + title_ch + char_h / 2;
        fb.draw_str(subtitle, sub_x, sub_y, scale, true);

        // Target below subtitle
        let target_str = format!("> {}", display_target);
        let target_x = (screen_w - target_str.len() * sub_cw) / 2;
        let target_y = sub_y + char_h * 2;
        fb.draw_str(&target_str, target_x, target_y, scale, true);

        fb.refresh_full();
        std::thread::sleep(std::time::Duration::from_millis(600));

        // Animate starfield — stars radiate outward from center
        // On e-ink, DU partial refreshes leave ghost trails = natural warp streaks
        let frame_ms = 80;
        let total_frames = 3000 / frame_ms; // ~3 seconds of warp

        for frame in 0..total_frames {
            // Draw stars as white dots/streaks on black background
            for star in &mut stars {
                let cos_a = cos_table[star.angle_idx];
                let sin_a = sin_table[star.angle_idx];

                // Streak: draw a short radial line from old to new position
                let streak_len = (star.speed * 2).min(star.radius / 3 + 1);
                let r_start = star.radius.saturating_sub(streak_len).max(0);
                let r_end = star.radius;

                // Draw streak (white pixels on black bg)
                let dot_size = if star.radius > max_r as i32 / 2 { 3 } else { 2 };
                let mut r = r_start;
                while r <= r_end {
                    let px = cx as i32 + (r * cos_a) / 1024;
                    let py = cy as i32 + (r * sin_a) / 1024;
                    // Draw a small block for visibility on e-ink
                    for dy in 0..dot_size {
                        for dx in 0..dot_size {
                            fb.set_pixel(
                                (px + dx) as usize,
                                (py + dy) as usize,
                                true, // white
                            );
                        }
                    }
                    r += 2;
                }

                // Advance star outward
                // Stars accelerate as they get farther (perspective effect)
                let accel = 1 + star.radius / (max_r as i32 / 3).max(1);
                star.radius += star.speed + accel;

                // Respawn at center when off screen
                if star.radius > max_r as i32 {
                    star.radius = 0;
                    star.angle_idx = (rng_next(&mut rng_state) % 256) as usize;
                    star.speed = 3 + (rng_next(&mut rng_state) % 12) as i32;
                }
            }

            // Redraw title text each frame (stars may overwrite it)
            fb.draw_str(title, title_x, title_y, title_scale, true);
            fb.draw_str(subtitle, sub_x, sub_y, scale, true);
            fb.draw_str(&target_str, target_x, target_y, scale, true);

            // DU partial refresh — fast 2-level update, ghosts create streak trails
            fb.refresh_fast(0, 0, screen_w as u32, screen_h as u32);

            std::thread::sleep(std::time::Duration::from_millis(frame_ms as u64));

            // Every 8 frames, full refresh to reset ghost buildup
            if frame % 8 == 7 {
                fb.refresh_full();
            }
        }

        // Final flash: white screen (arrival)
        fb.clear();
        fb.refresh_full();
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    // ---- Initial terminal render ----
    render_terminal(&mut fb, &term, scale);
    render_status_bar(&mut fb, &term, scale, term_area_height, display_target);

    if let Some(ref osk) = osk {
        osk.render(&mut fb, scale);
    }

    fb.refresh_full();

    // ---- Build pollfd array ----
    let mut poll_fds: Vec<sys::pollfd> = Vec::new();

    // Index 0: PTY master
    poll_fds.push(sys::pollfd {
        fd: pty.master_fd,
        events: sys::POLLIN,
        revents: 0,
    });

    // Index 1: physical keyboard (if available)
    let mut kb_poll_idx = if let Some(fd) = phys_kb.fd() {
        poll_fds.push(sys::pollfd {
            fd,
            events: sys::POLLIN,
            revents: 0,
        });
        Some(poll_fds.len() - 1)
    } else {
        None
    };

    // Index 2+: touch input devices
    let touch_poll_start = poll_fds.len();
    if let Some(ref ti) = touch_input {
        for fd in ti.fds() {
            poll_fds.push(sys::pollfd {
                fd,
                events: sys::POLLIN,
                revents: 0,
            });
        }
    }

    // ---- Main event loop ----
    let mut last_refresh = Instant::now();
    let mut needs_refresh = false;
    let mut last_status_cursor = (0usize, 0usize); // (cursor_y, cursor_x) for status bar change detection

    eprintln!("Entering main loop...");

    loop {
        if !pty.is_alive() {
            eprintln!("Child process exited.");
            break;
        }

        for pfd in &mut poll_fds {
            pfd.revents = 0;
        }

        let timeout_ms = if needs_refresh {
            REFRESH_DEBOUNCE_MS as sys::c_int
        } else {
            1000
        };

        let ret = unsafe {
            sys::poll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as sys::nfds_t,
                timeout_ms,
            )
        };

        // Check if a signal requested exit
        if SIGNAL_EXIT.load(Ordering::SeqCst) {
            eprintln!("Signal received, exiting...");
            break;
        }

        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue; // Signal interrupted poll; loop back to check SIGNAL_EXIT
            }
            eprintln!("poll error: {}", err);
            break;
        }

        // ---- Handle PTY output ----
        if poll_fds[0].revents & sys::POLLIN != 0 {
            let data = pty.read();
            if !data.is_empty() {
                term.process(&data);
                // Flush any terminal responses (DSR, DA) back to the PTY
                if let Some(response) = term.take_response() {
                    let _ = pty.write(&response);
                }
                needs_refresh = true;
            }
        }
        if poll_fds[0].revents & sys::POLLHUP != 0 {
            eprintln!("PTY closed.");
            break;
        }

        // ---- Handle physical keyboard input ----
        if let Some(idx) = kb_poll_idx {
            if poll_fds[idx].revents & sys::POLLHUP != 0 {
                eprintln!("Keyboard disconnected.");
                // Disable the fd so poll doesn't spin on the dead descriptor
                poll_fds[idx].fd = -1;
                kb_poll_idx = None;
            } else if poll_fds[idx].revents & sys::POLLIN != 0 {
                let key_events = phys_kb.read_keys(term.app_cursor_keys);
                for keys in key_events {
                    if let Err(e) = pty.write(&keys) {
                        eprintln!("Write to PTY failed: {}", e);
                    }
                }
            }
        }

        // ---- Handle touch input ----
        if let Some(ref mut ti) = touch_input {
            for i in touch_poll_start..poll_fds.len() {
                if poll_fds[i].revents & sys::POLLIN != 0 {
                    let fd_index = i - touch_poll_start;
                    let touches = ti.read_events(fd_index);
                    for touch in touches {
                        if let Some(ref mut osk) = osk {
                            if touch.pressure <= 0 {
                                // Finger lifted — reset debounce so next tap registers
                                osk.handle_touch_up();
                            } else if touch.y >= osk.origin_y as i32 {
                                if let Some(bytes) = osk.handle_touch(touch.x, touch.y) {
                                    if let Err(e) = pty.write(&bytes) {
                                        eprintln!("Write to PTY failed: {}", e);
                                    }
                                }
                                osk.render(&mut fb, scale);
                                let kb_y = osk.origin_y as u32;
                                fb.refresh_fast(
                                    0,
                                    kb_y,
                                    fb.width as u32,
                                    osk.total_height as u32,
                                );
                            }
                        }
                    }
                }
            }
        }

        // ---- Refresh display if needed ----
        if needs_refresh {
            let elapsed = last_refresh.elapsed().as_millis() as u64;
            if elapsed >= REFRESH_DEBOUNCE_MS || ret == 0 {
                render_terminal(&mut fb, &term, scale);
                render_status_bar(&mut fb, &term, scale, term_area_height, display_target);

                // Alt screen switch (vim/less/tmux enter/exit) — force full GC16
                // refresh to clear ghosting from the previous screen content.
                if term.needs_full_refresh {
                    term.needs_full_refresh = false;
                    term.mark_clean();
                    fb.refresh_full();
                    last_refresh = Instant::now();
                    needs_refresh = false;
                    continue;
                }

                let (min_row, min_col, max_row, max_col) = term.mark_clean();

                // Use dirty rect to refresh only the changed region
                if min_row <= max_row && min_col <= max_col {
                    let refresh_x = (min_col * char_w) as u32;
                    let refresh_y = (min_row * char_h) as u32;
                    let refresh_w = ((max_col - min_col + 1) * char_w) as u32;
                    let refresh_h = ((max_row - min_row + 1) * char_h) as u32;
                    let refresh_w = refresh_w.min((fb.width as u32).saturating_sub(refresh_x));
                    let refresh_h = refresh_h.min((fb.height as u32).saturating_sub(refresh_y));
                    fb.refresh_fast(refresh_x, refresh_y, refresh_w, refresh_h);
                    // Refresh status bar only if cursor position changed
                    let cur_pos = (term.cursor_y, term.cursor_x);
                    if cur_pos != last_status_cursor {
                        last_status_cursor = cur_pos;
                        fb.refresh_fast(
                            0,
                            term_area_height as u32,
                            fb.width as u32,
                            status_bar_height as u32,
                        );
                    }
                } else {
                    // Fallback: refresh full terminal area + status bar
                    fb.refresh_terminal(term_area_height as u32 + status_bar_height as u32);
                }

                last_refresh = Instant::now();
                needs_refresh = false;
            }
        }
    }

    // ---- Exit screen ----
    fb.clear();
    if fb.is_rm2fb() {
        // With rm2fb, xochitl is still running — just clear our drawing
        fb.refresh_full();
    } else {
        // Centered exit message
        let exit_msg = "Session ended.";
        let exit_cw = font::FONT_WIDTH * scale;
        let exit_x = (fb.width - exit_msg.len() * exit_cw) / 2;
        let exit_y = fb.height / 3;
        fb.draw_str(exit_msg, exit_x, exit_y, scale, false);
        fb.refresh_full();
        std::thread::sleep(std::time::Duration::from_secs(1));
        restart_xochitl();
    }

    eprintln!("Display backend was: {}", fb.backend_name());

    eprintln!("Exiting.");
}
