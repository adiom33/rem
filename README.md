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

The app automatically stops the normal reMarkable UI (called "xochitl") when it
starts, and **restarts it when you exit** -- even if the app crashes or you
press Ctrl+C. You will not get stuck with a blank screen.

## Step-by-Step Setup

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

### Step 3: Build the App

On your computer (not the reMarkable), you need Rust installed.
If you don't have it:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then clone this repository and build:

```bash
# Get the code
git clone <this-repo>
cd rem

# Install the ARM cross-compilation target
rustup target add armv7-unknown-linux-musleabihf

# Build (this creates a single static binary that runs on the reMarkable)
./scripts/build.sh
```

**If the build fails** with a linker error, you need an ARM cross-compiler.
The build script tries these methods in order:

1. **`cross` (easiest, recommended):** Uses Docker, no toolchain setup:
   ```bash
   cargo install cross
   # Then re-run: ./scripts/build.sh
   ```
2. **Ubuntu/Debian native:** Install a cross-linker:
   ```bash
   sudo apt install gcc-arm-linux-gnueabihf
   # The build script auto-detects this and uses it
   ```
3. **Mac:** `brew install filosottile/musl-cross/musl-cross`

### Step 4: Copy the App to Your reMarkable

With your reMarkable plugged in via USB:

```bash
./scripts/deploy.sh 10.11.99.1
# Enter your reMarkable password when prompted
```

This copies the compiled binary to `/home/root/remarkable-ssh` on the tablet.

### Step 5: Run It

**Option A: One command from your computer (easiest)**

```bash
./scripts/run-remote.sh 10.11.99.1 user@your-server-ip
```

This SSHes into the reMarkable, stops xochitl, runs the terminal, and restarts
xochitl when you're done. Everything is automatic.

**Option B: Run it manually on the reMarkable**

```bash
# SSH into your reMarkable
ssh root@10.11.99.1

# Run the terminal app (it stops/restarts xochitl automatically)
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
hardware EPDC. This means the app needs to tell the kernel to push pixels
to the e-ink panel after drawing them.

**On startup, the app probes your display driver** and logs what it finds:

```
MXCFB_SEND_UPDATE probe: V2 ioctl works (72-byte struct)   ← good, native refresh works
```

or:

```
WARNING: No working MXCFB_SEND_UPDATE ioctl found!         ← you need rm2fb
```

### If the screen stays blank: you need rm2fb

On most RM2 firmware, direct framebuffer ioctls don't trigger screen updates.
You need `rm2fb` to bridge between the framebuffer and the display.

**Step 1: Get rm2fb for your firmware**

Check the [remarkable2-framebuffer](https://github.com/ddvk/remarkable2-framebuffer)
releases. You need a version that matches your firmware (check Settings > General
> About on your tablet).

```bash
# On your computer -- download the release matching your firmware
wget https://github.com/ddvk/remarkable2-framebuffer/releases/latest/download/rm2fb.tar.gz
tar xzf rm2fb.tar.gz

# Copy to reMarkable
scp librm2fb_server.so.1.0.1 root@10.11.99.1:/opt/lib/
scp librm2fb_client.so.1.0.1 root@10.11.99.1:/opt/lib/

# On the reMarkable -- create symlinks
ssh root@10.11.99.1
mkdir -p /opt/lib
cd /opt/lib
ln -sf librm2fb_server.so.1.0.1 librm2fb_server.so.1
ln -sf librm2fb_client.so.1.0.1 librm2fb_client.so.1
```

**Step 2: Run with rm2fb**

```bash
# Start xochitl with the rm2fb server loaded
systemctl stop xochitl
LD_PRELOAD=/opt/lib/librm2fb_server.so.1 xochitl &

# Run remarkable-ssh with the rm2fb client
LD_PRELOAD=/opt/lib/librm2fb_client.so.1 /home/root/remarkable-ssh user@host
```

**If rm2fb says "Missing address for function":** Your firmware version isn't
supported yet. Check the rm2fb issues/wiki for your specific version, or try
an older firmware.

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
| **Screen stays blank / no refresh** | Check the startup log. If it says "No working MXCFB_SEND_UPDATE ioctl found", you need rm2fb. See [Display Setup](#display-setup-rm2-firmware-compatibility). |
| **"MXCFB V2 probe failed: ENOTTY"** | Your kernel uses a different ioctl struct size. The app auto-tries V1 and V2. If both fail, you need rm2fb. |
| **rm2fb: "Missing address for function"** | rm2fb doesn't support your firmware version. Check the rm2fb issues for your specific firmware. |
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
    framebuffer.rs         # Draws to the e-ink screen via /dev/fb0
    font.rs               # Built-in 8x16 pixel bitmap font (95 ASCII glyphs)
    terminal.rs           # VT100/xterm escape sequence parser
    keyboard.rs           # Physical keyboard + on-screen keyboard
    input.rs              # Touchscreen and pen input handling
    pty.rs                # Pseudo-terminal (PTY) and child process management
    sys.rs                # Raw Linux syscall bindings (replaces libc crate)
  scripts/
    build.sh              # Cross-compile for ARM (reMarkable hardware)
    deploy.sh             # Copy binary to reMarkable via USB
    run-remote.sh         # Build + deploy + run, all in one command
```

The entire app is a single Rust binary with **zero external dependencies**.
All system calls are made directly through inline bindings in `sys.rs`, and
the binary is statically linked via musl, so it runs on any reMarkable 2
without installing anything else.

## License

MIT
