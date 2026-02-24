#!/usr/bin/env bash
#
# install.sh — One-click install of remarkable-ssh on reMarkable 2
#
# Usage:
#   ./scripts/install.sh [TABLET_IP] [SSH_TARGET]
#
# Examples:
#   ./scripts/install.sh                                # defaults
#   ./scripts/install.sh 10.11.99.1                     # just tablet IP
#   ./scripts/install.sh 10.11.99.1 user@myserver       # saves SSH target for quick launch
#
# This script:
#   1. Builds the remarkable-ssh binary for ARM
#   2. Deploys it to the tablet
#   3. Runs --setup on the tablet to auto-extract rm2fb addresses
#   4. Checks if rm2fb server .so is present
#   5. Configures systemd to start xochitl with rm2fb
#   6. Restarts xochitl with rm2fb enabled
#   7. Deploys a 'term' launcher script for quick launch from phone/SSH
#
# After this, just run:
#   ./scripts/run-remote.sh TABLET_IP user@your-server
# Or from your phone:
#   ssh root@TABLET_IP ./term
#
set -euo pipefail

TABLET_IP="${1:-10.11.99.1}"
SSH_TARGET="${2:-}"
SSH_OPTS=(-o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)
REMOTE_BIN="/home/root/remarkable-ssh"
REMOTE_LAUNCHER="/home/root/term"
RM2FB_RELEASE_URL="https://github.com/ddvk/remarkable2-framebuffer/releases/latest/download"

run_remote() {
    ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "$@"
}

echo "=== remarkable-ssh One-Click Install ==="
echo ""

# ---- Preflight ----
echo "[preflight] Checking tablet connectivity..."
if ! run_remote 'echo ok' &>/dev/null; then
    echo "ERROR: Cannot reach tablet at $TABLET_IP"
    echo "  Make sure:"
    echo "  - The tablet is connected via USB"
    echo "  - SSH works: ssh root@$TABLET_IP"
    exit 1
fi

FIRMWARE_VER=$(run_remote 'grep REMARKABLE_RELEASE_VERSION /usr/share/remarkable/update.conf 2>/dev/null | cut -d= -f2 || cat /etc/version 2>/dev/null || echo unknown' | tr -d '[:space:]')
DEVICE_MODEL=$(run_remote 'cat /sys/firmware/devicetree/base/model 2>/dev/null | tr -d "\0" || echo unknown' | tr -d '[:space:]')
echo "  Device:   $DEVICE_MODEL"
echo "  Firmware: $FIRMWARE_VER"
echo ""

# ---- Step 1: Build ----
echo "[1/6] Building remarkable-ssh..."
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
"${SCRIPT_DIR}/build.sh" || {
    echo "ERROR: Build failed. See errors above."
    exit 1
}
echo ""

# ---- Step 2: Deploy binary ----
echo "[2/6] Deploying to tablet..."
"${SCRIPT_DIR}/deploy.sh" "$TABLET_IP" || {
    echo "ERROR: Deploy failed. See errors above."
    exit 1
}
echo ""

# ---- Step 3: Check if rm2fb server .so exists ----
echo "[3/6] Checking rm2fb server..."
HAS_SERVER=$(run_remote 'ls /opt/lib/librm2fb_server.so* 2>/dev/null | head -1 || echo ""')

if [ -z "$HAS_SERVER" ]; then
    echo "  rm2fb server .so not found on tablet."
    echo ""
    echo "  Attempting to download from GitHub releases..."

    # Try to download on the tablet directly (it has wget/curl sometimes)
    DOWNLOAD_OK=$(run_remote '
        mkdir -p /opt/lib
        cd /tmp
        # Try wget first, then curl
        if command -v wget &>/dev/null; then
            wget -q "https://github.com/ddvk/remarkable2-framebuffer/releases/latest/download/librm2fb_server.so.1.0.1" -O librm2fb_server.so.1.0.1 2>/dev/null
        elif command -v curl &>/dev/null; then
            curl -sL "https://github.com/ddvk/remarkable2-framebuffer/releases/latest/download/librm2fb_server.so.1.0.1" -o librm2fb_server.so.1.0.1 2>/dev/null
        else
            echo "NO_DOWNLOADER"
            exit 1
        fi

        if [ -f librm2fb_server.so.1.0.1 ] && [ -s librm2fb_server.so.1.0.1 ]; then
            mv librm2fb_server.so.1.0.1 /opt/lib/
            cd /opt/lib
            ln -sf librm2fb_server.so.1.0.1 librm2fb_server.so.1
            echo "OK"
        else
            echo "DOWNLOAD_FAILED"
        fi
    ' 2>/dev/null || echo "FAILED")

    case "$DOWNLOAD_OK" in
        *OK*)
            echo "  Downloaded and installed rm2fb server .so"
            HAS_SERVER="yes"
            ;;
        *NO_DOWNLOADER*)
            echo "  No wget/curl on tablet. Download manually:"
            echo "    wget $RM2FB_RELEASE_URL/librm2fb_server.so.1.0.1"
            echo "    scp librm2fb_server.so.1.0.1 root@${TABLET_IP}:/opt/lib/"
            echo "    ssh root@${TABLET_IP} 'cd /opt/lib && ln -sf librm2fb_server.so.1.0.1 librm2fb_server.so.1'"
            ;;
        *)
            echo "  Download failed (tablet may not have internet access)."
            echo "  Download on your computer and deploy:"
            echo "    wget $RM2FB_RELEASE_URL/librm2fb_server.so.1.0.1"
            echo "    scp librm2fb_server.so.1.0.1 root@${TABLET_IP}:/opt/lib/"
            echo "    ssh root@${TABLET_IP} 'cd /opt/lib && ln -sf librm2fb_server.so.1.0.1 librm2fb_server.so.1'"
            ;;
    esac
else
    echo "  Found: $HAS_SERVER"
fi
echo ""

# ---- Step 4: Auto-extract rm2fb addresses ----
echo "[4/6] Extracting rm2fb function addresses from xochitl..."
echo "  Running: remarkable-ssh --setup"
echo ""

SETUP_RESULT=$(run_remote "$REMOTE_BIN --setup" 2>&1) || true
echo "$SETUP_RESULT" | sed 's/^/  /'
echo ""

if echo "$SETUP_RESULT" | grep -q "rm2fb.conf written successfully"; then
    echo "  Addresses extracted and rm2fb.conf written!"
else
    echo "  WARNING: Auto-extraction may have failed."
    echo "  Check the output above. If addresses were not found,"
    echo "  you may need to extract them manually with Ghidra."
    echo "  See: ./scripts/extract-rm2fb-addrs.sh"
    echo ""
    # Don't exit — let the user see the state and decide
fi

# ---- Step 5: Configure systemd and restart xochitl ----
echo "[5/6] Configuring systemd..."

# Check that we have both the .conf and the .so before proceeding
HAS_CONF=$(run_remote 'test -f /etc/rm2fb.conf && echo yes || echo no')
HAS_SERVER=$(run_remote 'ls /opt/lib/librm2fb_server.so* 2>/dev/null | head -1 || echo ""')

if [ "$HAS_CONF" = "yes" ] && [ -n "$HAS_SERVER" ]; then
    # Create systemd override
    run_remote '
        mkdir -p /etc/systemd/system/xochitl.service.d
        cat > /etc/systemd/system/xochitl.service.d/rm2fb.conf <<UNIT
[Service]
Environment=LD_PRELOAD=/opt/lib/librm2fb_server.so.1
UNIT
        systemctl daemon-reload
    '
    echo "  Systemd override created."

    echo "  Restarting xochitl with rm2fb..."
    run_remote 'systemctl restart xochitl' 2>/dev/null || true

    # Wait for xochitl to start and rm2fb to create shared memory
    echo "  Waiting for rm2fb to initialize..."
    sleep 5

    RM2FB_RUNNING=$(run_remote 'test -e /dev/shm/swtfb.01 && echo yes || echo no')
    if [ "$RM2FB_RUNNING" = "yes" ]; then
        echo "  rm2fb is running! (/dev/shm/swtfb.01 exists)"
    else
        echo "  WARNING: /dev/shm/swtfb.01 not found after restart."
        echo "  rm2fb may have failed to start. Check with:"
        echo "    ssh root@$TABLET_IP 'journalctl -u xochitl -n 20'"
        echo ""
        echo "  Common issues:"
        echo "  - 'Missing address for function': addresses in rm2fb.conf are wrong"
        echo "  - Crash/segfault: rm2fb .so is ABI-incompatible with this firmware"
        echo "  - Qt errors: rm2fb was built against a different Qt version"
    fi
else
    echo "  Skipping systemd config (missing rm2fb.conf or server .so)."
    echo "  Fix the issues above and re-run this script."
fi

# ---- Step 6: Deploy launcher script ----
echo ""
echo "[6/6] Deploying launcher script..."

# Build the launcher script content
# If the user gave us an SSH target, bake it in as the default
if [ -n "$SSH_TARGET" ]; then
    LAUNCHER_DEFAULT="$SSH_TARGET"
else
    LAUNCHER_DEFAULT=""
fi

# Generate the launcher script locally and deploy via stdin
ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "cat > $REMOTE_LAUNCHER && chmod +x $REMOTE_LAUNCHER" <<LAUNCHER_EOF
#!/bin/sh
# Quick launcher for remarkable-ssh.
# Run:  ./term                   (uses saved default target)
#       ./term user@otherhost    (override target)
#       ./term --keyboard        (with on-screen keyboard)
#
# Edit DEFAULT_TARGET below to change your default server.

DEFAULT_TARGET="${LAUNCHER_DEFAULT}"

# If the user passed arguments, use them. Otherwise use the default.
if [ \$# -gt 0 ]; then
    exec /home/root/remarkable-ssh "\$@"
elif [ -n "\$DEFAULT_TARGET" ]; then
    exec /home/root/remarkable-ssh "\$DEFAULT_TARGET"
else
    echo "Usage: ./term [user@host] [OPTIONS]"
    echo ""
    echo "No default SSH target configured."
    echo "Edit /home/root/term and set DEFAULT_TARGET, or pass a target:"
    echo "  ./term user@your-server"
    exit 1
fi
LAUNCHER_EOF

if [ -n "$SSH_TARGET" ]; then
    echo "  Launcher deployed: $REMOTE_LAUNCHER (default: $SSH_TARGET)"
else
    echo "  Launcher deployed: $REMOTE_LAUNCHER (no default target)"
    echo "  To set a default: ssh root@$TABLET_IP and edit $REMOTE_LAUNCHER"
fi

echo ""
echo "=== Install Complete ==="
echo ""
echo "To use from your computer:"
echo "  ./scripts/run-remote.sh $TABLET_IP user@your-server-ip"
echo ""
echo "To use from your phone (no computer needed):"
echo "  ssh root@$TABLET_IP ./term"
if [ -n "$SSH_TARGET" ]; then
    echo "  (will connect to $SSH_TARGET by default)"
fi
echo ""
echo "To use with options:"
echo "  ./scripts/run-remote.sh $TABLET_IP user@server --tmux --keyboard"
echo ""
echo "To undo rm2fb (restore stock xochitl):"
echo "  ssh root@$TABLET_IP 'rm /etc/systemd/system/xochitl.service.d/rm2fb.conf && systemctl daemon-reload && systemctl restart xochitl'"
