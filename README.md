# remarkable-ssh

A framebuffer-based SSH terminal for the **reMarkable 2** tablet.

Turns your reMarkable 2 into a distraction-free SSH thin client, connecting over
Tailscale (or any network) to your home server — while still functioning as an
e-book reader when you're done.

## What This Does

- Opens the e-ink framebuffer directly (`/dev/fb0`)
- Renders a VT100-compatible terminal with an embedded bitmap font
- Reads input from the **Type Folio physical keyboard** (evdev)
- Optionally shows an on-screen virtual keyboard (for touch-only use)
- Spawns SSH (or a local shell) via a PTY
- Uses fast partial e-ink refreshes for responsive typing
- Ships as a **single static binary** with zero dependencies

## Architecture

```
┌─────────────────────────────────────────┐
│  reMarkable 2                           │
│                                         │
│  ┌──────────┐   ┌──────────────────┐    │
│  │ Type     │──>│ remarkable-ssh    │    │       ┌──────────────┐
│  │ Folio KB │   │                  │    │       │ Home Server  │
│  └──────────┘   │  evdev ──> PTY ──────────────>│              │
│                 │  VT100 <── PTY <──────────────│  (Tailscale) │
│  ┌──────────┐   │            │      │    │       │  100.64.x.x  │
│  │ e-ink    │<──│  framebuffer      │    │       └──────────────┘
│  │ display  │   │  /dev/fb0  │      │    │
│  └──────────┘   └──────────────────┘    │
│                                         │
│  Tailscale NOT needed on reMarkable.    │
│  Just SSH to the Tailscale IP.          │
└─────────────────────────────────────────┘
```

## Prerequisites

**On your development machine:**
- Rust toolchain: https://rustup.rs/
- One of:
  - `cross` (recommended): `cargo install cross` (requires Docker)
  - ARM cross-compiler: `apt install gcc-arm-linux-gnueabihf musl-tools`

**On the reMarkable 2:**
- SSH access enabled (Settings > General > Help > Copyrights and licenses → password shown)
- For rM2 display: `rm2fb` shim (see [Display Setup](#display-setup-rm2))

**On your home server:**
- Tailscale running (the reMarkable does NOT need Tailscale)
- SSH server running

## Quick Start

```bash
# 1. Clone this repo
git clone <this-repo> && cd rem

# 2. Build for reMarkable (ARM static binary)
./scripts/build.sh

# 3. Deploy to reMarkable via USB
./scripts/deploy.sh 10.11.99.1

# 4. SSH into reMarkable and run
ssh root@10.11.99.1
systemctl stop xochitl
/home/root/remarkable-ssh user@100.64.0.1    # your Tailscale IP
systemctl start xochitl                      # when done
```

Or use the all-in-one script:
```bash
./scripts/run-remote.sh 10.11.99.1 user@100.64.0.1
```

## Usage

```
remarkable-ssh [OPTIONS] [user@host]

OPTIONS:
  --scale N       Font scale factor (default: 2, gives 16x32 pixel chars)
  --fb PATH       Framebuffer device (default: /dev/fb0)
  --shell CMD     Shell to run if no SSH target (default: /bin/sh)
  --keyboard      Enable on-screen virtual keyboard
  --ssh-cmd CMD   SSH client binary (default: ssh, fallback: dbclient)
  --help          Show help

EXAMPLES:
  remarkable-ssh user@192.168.1.100         # SSH via local network
  remarkable-ssh user@100.64.0.1            # SSH via Tailscale
  remarkable-ssh --keyboard user@myhost     # With on-screen keyboard
  remarkable-ssh --shell /bin/bash          # Local shell (no SSH)
  remarkable-ssh --scale 3 user@host        # Larger font
```

## Display Setup (rM2)

The reMarkable 2 doesn't expose a traditional framebuffer — the display is driven
by a software controller (SWTCON). You need the `rm2fb` shim to make `/dev/fb0` work.

### Installing rm2fb manually (no Toltec needed)

```bash
# On your development machine, download the rm2fb release:
wget https://github.com/ddvk/remarkable2-framebuffer/releases/latest/download/rm2fb.tar.gz
tar xzf rm2fb.tar.gz

# Copy to reMarkable:
scp librm2fb_server.so.1.0.1 root@10.11.99.1:/opt/lib/
scp librm2fb_client.so.1.0.1 root@10.11.99.1:/opt/lib/

# On the reMarkable:
ssh root@10.11.99.1
cd /opt/lib
ln -s librm2fb_server.so.1.0.1 librm2fb_server.so.1
ln -s librm2fb_client.so.1.0.1 librm2fb_client.so.1

# Run remarkable-ssh with rm2fb:
systemctl stop xochitl
LD_PRELOAD=/opt/lib/librm2fb_client.so.1 /home/root/remarkable-ssh user@host
systemctl start xochitl
```

If rm2fb is not available for your firmware version, check the
[remarkable2-framebuffer](https://github.com/ddvk/remarkable2-framebuffer) repo
for alternatives.

## Keyboard Support

### Type Folio (physical keyboard)
The Type Folio keyboard is supported out of the box. It sends standard Linux
key events which are mapped to terminal sequences:

- All letters, numbers, symbols
- Arrow keys, Home, End, Page Up/Down
- F1-F12
- Ctrl+C, Ctrl+D, Ctrl+Z, Ctrl+L (all Ctrl combos)
- Alt+key (sends ESC prefix)
- Tab, Escape, Backspace, Delete

### On-Screen Keyboard
Enable with `--keyboard` flag. Tap keys on the touchscreen.
Includes Shift, Ctrl, and all terminal-essential keys.

## Terminal Capabilities

The built-in terminal emulator supports:

- **Cursor movement**: up/down/left/right, absolute positioning
- **Erase**: clear screen, clear line, clear to end/beginning
- **Scrolling**: scroll regions, insert/delete lines
- **Text attributes**: bold (rendered normal), inverse video
- **Application cursor keys** (for vim, less, etc.)
- **Line wrapping**
- **Tab stops** (every 8 columns)
- **TERM=xterm** (good compatibility with most servers)

This is enough for: bash, zsh, vim, nano, less, htop, tmux, screen, and most
CLI tools.

## E-Ink Refresh Strategy

E-ink displays are slow. This app uses:

- **DU waveform** for fast partial updates (typing, cursor movement)
- **GC16 waveform** for periodic full refreshes (clears ghosting)
- **Debounced refreshes**: waits 50ms after output stops before refreshing,
  so rapid scrolling doesn't trigger individual refreshes per line

## Returning to Normal reMarkable Use

The reMarkable's normal UI (xochitl) is stopped while the terminal runs.
When you exit (or the SSH session ends), restart it:

```bash
systemctl start xochitl
```

The `run-remote.sh` script does this automatically.

## Project Structure

```
├── Cargo.toml              # Rust project config
├── .cargo/config.toml      # Cross-compilation settings
├── src/
│   ├── main.rs             # Entry point, event loop, rendering
│   ├── framebuffer.rs      # /dev/fb0 mmap, pixel drawing, e-ink refresh
│   ├── font.rs             # Embedded 8x16 bitmap font (VGA style)
│   ├── input.rs            # evdev touch/pen input handling
│   ├── keyboard.rs         # Type Folio + on-screen keyboard
│   ├── terminal.rs         # VT100 escape sequence parser + grid
│   └── pty.rs              # PTY management, child process spawning
├── scripts/
│   ├── build.sh            # Cross-compile for ARM
│   ├── deploy.sh           # SCP binary to reMarkable
│   └── run-remote.sh       # Build + deploy + run (all-in-one)
└── README.md
```

## Troubleshooting

**"Cannot open /dev/fb0"**: Make sure xochitl is stopped (`systemctl stop xochitl`).

**Display not updating (rM2)**: You need rm2fb. See [Display Setup](#display-setup-rm2).

**No keyboard input**: Check that the Type Folio is connected. The app tries
`/dev/input/event0` through `event5`. If your keyboard is on a different device,
check `cat /proc/bus/input/devices` on the reMarkable.

**SSH connection refused**: Make sure the target server has SSH running and is
reachable. If using Tailscale, verify the Tailscale IP on the server with
`tailscale ip -4`.

**"command not found: ssh"**: The reMarkable may only have `dbclient` (dropbear
SSH client). Use `--ssh-cmd dbclient` or just let it auto-detect.

## License

MIT
