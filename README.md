# remarkable-ssh

A terminal app for the **reMarkable 2** tablet that lets you SSH into another computer.

Think of it as turning your reMarkable into a portable, distraction-free terminal
that connects to your server, home computer, or cloud machine. When you're done,
your reMarkable goes right back to being an e-reader.

## What You Need

**Three things:**

1. **A reMarkable 2 tablet** (with optional Type Folio keyboard, or use the on-screen keyboard)
2. **A computer to build and copy the app** (Linux, Mac, or Windows with WSL)
3. **A machine to SSH into** (your home server, a VPS, a Raspberry Pi, etc.)

## How It Works

Your reMarkable connects to the target machine over SSH (the same way you'd
connect from a regular terminal). The app draws directly to the e-ink screen
and reads your keypresses, so no special display server or desktop environment
is needed.

```
reMarkable 2                          Your Server
+------------------+                  +--------------+
| Type Folio KB    |    SSH over      |              |
| or on-screen KB  |----- WiFi ------>| bash, tmux,  |
|                  |                  | vim, htop... |
| e-ink display    |<----- WiFi -----|              |
| (terminal view)  |                  +--------------+
+------------------+
```

The app automatically handles the reMarkable UI (called "xochitl"):
- **Without rm2fb:** Stops xochitl on start, **restarts it when you exit** -- even
  if the app crashes or you press Ctrl+C. You will not get stuck with a blank screen.
- **With rm2fb:** Leaves xochitl running (the rm2fb server needs it). See
  [Display Setup](#display-setup-rm2-firmware-compatibility) if your screen stays blank.

## Quick Start

If you have Rust and Docker installed:

```bash
git clone https://github.com/adiom33/rem.git
cd rem
cargo install cross                       # Docker-based ARM cross-compiler
./scripts/install.sh 10.11.99.1           # Build, deploy, auto-configure rm2fb
./scripts/run-remote.sh 10.11.99.1 user@your-server-ip
```

`install.sh` does everything: builds the binary, deploys it, auto-extracts
rm2fb addresses from your tablet's xochitl binary, writes the config, and
sets up the display backend. After that, `run-remote.sh` is all you need.

First time? Read on for the detailed setup.

## Detailed Setup

### Step 1: Find Your reMarkable's Password

On your reMarkable tablet:

1. Tap **Settings** (gear icon)
2. Tap **General**
3. Tap **Help**
4. Tap **Copyrights and licenses**
5. At the bottom, you'll see your **root password** and **IP address**

Write down the password. You'll need it to copy files to the tablet.

### Step 2: Connect Your reMarkable to Your Computer

Plug the reMarkable into your computer with the USB-C cable. This creates
a direct network connection with the IP address `10.11.99.1`.

To verify it works, open a terminal on your computer and run:

```bash
ssh root@10.11.99.1
# Enter the password from Step 1 when prompted
# Type 'exit' to disconnect
```

### Step 3: Install Rust and Build

You need two things on your computer: **Rust** and **`cross`** (a Docker-based
ARM cross-compiler). This is a one-time setup.

```bash
# Install Rust (if you don't have it)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install cross (uses Docker — handles all ARM toolchain stuff for you)
cargo install cross
```

> **Don't have Docker?** You can also build without it:
> - Ubuntu/Debian: `sudo apt install gcc-arm-linux-gnueabihf`
> - Mac: `brew install filosottile/musl-cross/musl-cross`
>
> The build script auto-detects whatever you have installed.

Then clone and build:

```bash
git clone https://github.com/adiom33/rem.git
cd rem
./scripts/build.sh
```

The build script handles everything — picks the right compiler, adds the ARM
target, and produces a static binary. No manual `rustup target add` needed.

### Step 4: Run It

**Option A: All-in-one (build + deploy + run)**

```bash
# Plug in reMarkable via USB, then:
./scripts/run-remote.sh 10.11.99.1 user@your-server-ip

# With extras:
./scripts/run-remote.sh 10.11.99.1 user@your-server-ip --tmux --keyboard
```

This does everything: builds, copies to the tablet, SSHes in, runs the terminal,
and restarts the reMarkable UI when you're done. You can append any flags after
the server address.

**Option B: Deploy and run separately**

```bash
# Copy the binary to the tablet
./scripts/deploy.sh 10.11.99.1

# SSH into your reMarkable and run it
ssh root@10.11.99.1
/home/root/remarkable-ssh user@your-server-ip
```

When you exit the SSH session (type `exit` or press Ctrl+D), the app
cleans up and restarts the normal reMarkable UI automatically.

## Usage

```
remarkable-ssh [OPTIONS] [user@host]
```

### Common Examples

```bash
# Basic SSH connection
remarkable-ssh user@192.168.1.100

# SSH with the on-screen keyboard (no Type Folio needed)
remarkable-ssh --keyboard user@myhost

# SSH and auto-attach to a tmux session (great for persistent sessions)
remarkable-ssh --tmux user@myhost

# Run a specific command on the remote machine
remarkable-ssh --cmd htop user@myhost

# Use a specific SSH key
remarkable-ssh --ssh-arg -i --ssh-arg /path/to/key user@myhost

# Just a local shell (no SSH, for testing on the device)
remarkable-ssh --shell /bin/sh

# Larger font (if you prefer bigger text)
remarkable-ssh --scale 3 user@myhost
```

### All Options

| Option | Description |
|--------|-------------|
| `--keyboard` | Show an on-screen virtual keyboard (for use without Type Folio) |
| `--kb PATH` | Use a specific keyboard device (e.g. `/dev/input/event3`) |
| `--tmux` | Auto-attach to a tmux session on the remote machine |
| `--cmd STRING` | Run a specific command remotely instead of a shell |
| `--scale N` | Font scale factor (default: 2, which gives 16x32 pixel characters) |
| `--fb PATH` | Framebuffer device path (default: `/dev/fb0`) |
| `--shell CMD` | Shell to use when no SSH target is given (default: `/bin/sh`) |
| `--ssh-cmd CMD` | SSH binary to use (default: `ssh`, falls back to `dbclient`) |
| `--ssh-arg ARG` | Extra argument to pass to SSH (repeatable) |
| `--ssh-args ARGS` | Extra arguments for SSH, comma-separated |
| `--setup` | Auto-extract rm2fb addresses and write `/etc/rm2fb.conf` (run on tablet) |
| `--help` | Show help |

### About `--tmux`

[tmux](https://github.com/tmux/tmux) is a "terminal multiplexer" that keeps
your session alive even if you disconnect. This is perfect for the reMarkable
because you can:

1. Start a session: `remarkable-ssh --tmux user@server`
2. Do some work, then exit
3. Come back later and your session is exactly where you left it

The `--tmux` flag automatically runs `tmux attach || tmux new -s main` on the
remote machine, so it reconnects to an existing session or creates a new one.

## Keyboard Support

### Type Folio (Physical Keyboard)

The Type Folio keyboard works automatically. The app auto-detects it by
scanning input devices. If auto-detection picks the wrong device, specify
it manually:

```bash
# Find your keyboard device
cat /proc/bus/input/devices   # look for "Type Folio" or similar

# Specify it
remarkable-ssh --kb /dev/input/event3 user@host
```

**Supported keys:** All letters, numbers, symbols, arrow keys, Home, End,
Page Up/Down, F1-F12, Tab, Escape, Backspace, Delete, plus all Ctrl and
Alt combinations.

### On-Screen Keyboard

If you don't have a Type Folio, use `--keyboard` to get a virtual keyboard
at the bottom of the screen. Tap keys to type. Special keys include:

- **SH** = Shift (tap once, then a letter)
- **CT** = Ctrl (tap once, then a letter -- e.g. CT then C = Ctrl+C)
- **ES** = Escape
- **TB** = Tab
- **EN** = Enter
- **BS** = Backspace

## Display Setup (rM2 Firmware Compatibility)

The reMarkable 2 uses a software display controller (SWTCON) instead of a
hardware EPDC. The app automatically detects the best way to update the screen:

| Backend | How it works | When it's used |
|---------|-------------|----------------|
| **Native MXCFB** | Direct ioctl to `/dev/fb0` | RM1, or RM2 with working EPDC driver |
| **rm2fb (auto)** | Shared memory + message queue | RM2 with rm2fb server running |
| **FBIOPAN (fallback)** | Raw framebuffer pan | RM2 without rm2fb — degraded quality |
| **None** | Screen won't update | Nothing works — see below |

**On startup, the app logs which backend it selected** (one of these):

```
Display backend: rm2fb                ← best on RM2
Display backend: native MXCFB V2     ← best on RM1
Display backend: FBIOPAN (degraded)   ← works but ugly, no rm2fb
=== NO WORKING DISPLAY BACKEND ===   ← need to set up rm2fb
```

It also logs device info for diagnostics:

```
Device: reMarkable 2
Firmware: 3.25.1.1
Framebuffer: 1404x1872 @ 16bpp (5136KB) driver="mxs-lcdif"
```

> **Note on the FBIOPAN fallback:** This pushes pixels through the LCD controller
> without e-ink waveform processing. Text may appear faint or require multiple
> refreshes. It exists as a last resort for testing. For usable output on RM2,
> install rm2fb.

### rm2fb: automatic integration (no LD_PRELOAD needed)

The app speaks the rm2fb protocol natively. If the rm2fb server is running,
the app auto-detects its shared memory (`/dev/shm/swtfb.01`) and message
queue, and uses them directly. **No `LD_PRELOAD` required on our side.**

You only need to get the rm2fb **server** running. The app handles the rest.

**Step 1: Install the rm2fb server**

Check the [remarkable2-framebuffer](https://github.com/ddvk/remarkable2-framebuffer)
releases for a version matching your firmware (Settings > General > About).

```bash
# On your computer
wget https://github.com/ddvk/remarkable2-framebuffer/releases/latest/download/rm2fb.tar.gz
tar xzf rm2fb.tar.gz

# Copy the SERVER library to the reMarkable (client .so is NOT needed)
scp librm2fb_server.so.1.0.1 root@10.11.99.1:/opt/lib/
ssh root@10.11.99.1 'mkdir -p /opt/lib && ln -sf librm2fb_server.so.1.0.1 /opt/lib/librm2fb_server.so.1'
```

**Step 2: Start xochitl with rm2fb server, then run the terminal**

```bash
# Restart xochitl with the rm2fb server loaded
systemctl stop xochitl
LD_PRELOAD=/opt/lib/librm2fb_server.so.1 xochitl &

# Just run — the app auto-detects rm2fb, no LD_PRELOAD needed
/home/root/remarkable-ssh user@host
```

**If rm2fb says "Missing address for function":** Your firmware version isn't
supported by that rm2fb release. Check the rm2fb issues/wiki for your version.

### Unsupported firmware? (3.4+)

Pre-built rm2fb only has addresses for firmware up to ~3.3. But `install.sh`
handles this automatically — it runs `remarkable-ssh --setup` on the tablet,
which parses the xochitl binary, finds the function addresses, and writes
`/etc/rm2fb.conf`. No Ghidra or manual reverse engineering needed.

If the auto-extraction fails (e.g. firmware changed the marker strings),
fall back to the manual scripts:

```bash
./scripts/rm2fb-check.sh 10.11.99.1          # Diagnose compatibility
./scripts/extract-rm2fb-addrs.sh 10.11.99.1   # Pull xochitl for Ghidra analysis
./scripts/deploy-rm2fb-conf.sh 10.11.99.1 0x<update> 0x<create>  # Deploy manually
./scripts/build-rm2fb.sh 10.11.99.1           # Build rm2fb from source (if .so is incompatible)
```

## What Works in the Terminal

The built-in terminal emulator is compatible with most command-line programs:

- **Shells**: bash, zsh, sh, fish
- **Editors**: vim, nano, micro
- **Multiplexers**: tmux, screen (with full alternate-screen support)
- **System tools**: htop, top, less, man, journalctl
- **General**: git, make, python, node, and most CLI tools

**Terminal features supported:**
- Cursor movement and positioning
- Screen and line clearing
- Scroll regions
- Inverse video (used for status bars, selections, etc.)
- Alternate screen buffer (so vim/tmux/less switch cleanly)
- Cursor show/hide
- Application cursor keys (arrow keys in vim, etc.)
- UTF-8 output (non-ASCII characters display as `?` -- the font is ASCII-only)

## E-Ink Tips

E-ink displays refresh differently from normal screens:

- **Typing and cursor movement** use fast partial refreshes (minimal flicker)
- **Every 20 updates**, a full refresh clears any ghosting (brief full-screen flash)
- **Fast scrolling** is batched -- the display waits 50ms for output to settle
  before refreshing, so scrolling doesn't flash once per line
- **Only the changed region** is refreshed, not the whole screen

If ghosting bothers you, press Ctrl+L to trigger a full redraw in most shells.

## Troubleshooting

| Problem | Solution |
|---------|----------|
| **Build fails: linker not found** | Install `cross` (`cargo install cross`) or `gcc-arm-linux-gnueabihf` (`sudo apt install gcc-arm-linux-gnueabihf`). The build script auto-detects available linkers. |
| **Build fails: `c_char` type error** | Make sure you have the latest code. The `c_char` type was changed from `i8` to `core::ffi::c_char` to work correctly on ARM targets. |
| **"Cannot open /dev/fb0"** | The app must run directly on the reMarkable, not over SSH in a normal terminal. Also make sure xochitl isn't holding the framebuffer -- the app stops it automatically, but if something went wrong, run `systemctl stop xochitl` first. |
| **Screen stays blank / no refresh** | Check the startup log. The app logs `Device:`, `Firmware:`, `Display backend:` on start. If it says "NO WORKING DISPLAY BACKEND", you need the rm2fb server. If it says "FBIOPAN (fallback)", the display will be degraded — install rm2fb for proper output. See [Display Setup](#display-setup-rm2-firmware-compatibility). |
| **rm2fb: "Missing address for function"** | The rm2fb server doesn't support your firmware version. Each firmware builds xochitl at different memory addresses, and rm2fb needs matching offsets. Either check the [rm2fb releases](https://github.com/ddvk/remarkable2-framebuffer/releases) or extract addresses yourself with `./scripts/rm2fb-check.sh` — see [Unsupported firmware](#unsupported-firmware-34). |
| **rm2fb: "Failed to lock epframebuffer"** | Another process already has the display lock. Stop xochitl first: `systemctl stop xochitl`, then start it with the rm2fb server: `LD_PRELOAD=/opt/lib/librm2fb_server.so.1 xochitl &` |
| **rm2fb: Qt library errors** | The rm2fb server binary was built against a different Qt version than your firmware. You need an rm2fb build that matches your firmware's Qt libraries. Check `/usr/lib/libQt5*.so*` on the tablet and compare with the rm2fb build requirements. |
| **Display is faint/degraded** | You're likely using the FBIOPAN fallback (check startup log). This bypasses e-ink waveform processing. Install rm2fb for proper display output. |
| **No keyboard input** | Make sure the Type Folio is connected. Try `--kb /dev/input/event3` (or event2, event4). Run `cat /proc/bus/input/devices` on the reMarkable to find the right device. |
| **SSH connection refused** | Make sure the target server has SSH running and is reachable from the reMarkable's network. |
| **"command not found: ssh"** | The reMarkable may only have `dbclient` (Dropbear SSH client). The app auto-detects this, but you can also use `--ssh-cmd dbclient`. |
| **Stuck on blank screen** | The app should restart xochitl automatically. If it didn't, SSH into your reMarkable and run `systemctl start xochitl`. |
| **Characters show as `?`** | The built-in font only covers ASCII (English letters, numbers, symbols). Non-ASCII characters (accented letters, emoji, CJK) show as `?`. |
| **SSH host key changed** | If you reimaged or updated firmware, remove the old key: `ssh-keygen -R 10.11.99.1` |

## Project Structure

```
rem/
  Cargo.toml              # Rust project configuration (zero dependencies)
  .cargo/config.toml      # Cross-compilation linker settings
  src/
    main.rs               # Entry point, event loop, display refresh
    framebuffer.rs         # E-ink display (native ioctls or rm2fb auto-detected)
    font.rs               # Built-in 8x16 pixel bitmap font (95 ASCII glyphs)
    terminal.rs           # VT100/xterm escape sequence parser
    keyboard.rs           # Physical keyboard + on-screen keyboard
    input.rs              # Touchscreen and pen input handling
    pty.rs                # Pseudo-terminal (PTY) and child process management
    sys.rs                # Raw Linux syscall bindings (replaces libc crate)
  scripts/
    install.sh            # One-click: build + deploy + auto-configure rm2fb
    build.sh              # Cross-compile for ARM (reMarkable hardware)
    deploy.sh             # Copy binary to reMarkable via USB
    run-remote.sh         # Build + deploy + run, all in one command
    rm2fb-check.sh        # Diagnose rm2fb compatibility on the tablet
    extract-rm2fb-addrs.sh # Pull xochitl and find rm2fb function addresses
    deploy-rm2fb-conf.sh  # Deploy custom rm2fb.conf with your addresses
    build-rm2fb.sh        # Build rm2fb from source for unsupported firmware
```

The entire app is a single Rust binary with **zero external dependencies**.
All system calls are made directly through inline bindings in `sys.rs`, and
the binary is statically linked via musl, so it runs on any reMarkable 2
without installing anything else.

## License

MIT
