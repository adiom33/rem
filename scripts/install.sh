#!/usr/bin/env bash
#
# install.sh — One-click install of remarkable-ssh on reMarkable 2
#
# Usage:
#   ./scripts/install.sh [TABLET_IP]
#
# This script:
#   1. Builds the remarkable-ssh binary for ARM
#   2. Deploys it to the tablet
#   3. Checks if rm2fb server .so is present
#   4. Auto-extracts rm2fb addresses from xochitl
#   5. Configures systemd for rm2fb + xochitl
#   6. Deploys launcher, systemd services, and configures boot mode
#   7. (Optional) Installs Tailscale for remote access
#
set -euo pipefail

TABLET_IP="${1:-10.11.99.1}"
SSH_OPTS=(-o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)
REMOTE_BIN="/home/root/remarkable-ssh"
REMOTE_LAUNCHER="/home/root/term"
RM2FB_RELEASE_URL="https://github.com/ddvk/remarkable2-framebuffer/releases/latest/download"

run_remote() {
    ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "$@"
}

# ---- Tailscale install function ----
install_tailscale() {
    echo ""
    echo "--- Installing Tailscale ---"
    echo ""

    # Detect tablet arch (should be armv7l)
    ARCH=$(run_remote 'uname -m' 2>/dev/null || echo "armv7l")
    case "$ARCH" in
        armv7*|armhf) TS_ARCH="arm" ;;
        aarch64)      TS_ARCH="arm64" ;;
        *)            TS_ARCH="arm" ;;
    esac

    # Get latest version from pkgs.tailscale.com
    echo "  Detecting latest Tailscale version..."
    TS_VERSION=$(curl -s https://pkgs.tailscale.com/stable/ | grep -oP 'tailscale_\K[0-9]+\.[0-9]+\.[0-9]+' | sort -V | tail -1 || echo "")
    if [ -z "$TS_VERSION" ]; then
        echo "  Could not detect latest version. Using 1.78.1"
        TS_VERSION="1.78.1"
    fi
    echo "  Latest version: $TS_VERSION"

    TS_TARBALL="tailscale_${TS_VERSION}_${TS_ARCH}.tgz"
    TS_URL="https://pkgs.tailscale.com/stable/${TS_TARBALL}"

    echo "  Downloading $TS_TARBALL..."

    # Try downloading on the tablet first, fall back to local download + SCP
    DOWNLOAD_RESULT=$(run_remote "
        cd /tmp
        rm -f $TS_TARBALL
        if command -v wget &>/dev/null; then
            wget -q '$TS_URL' -O '$TS_TARBALL' 2>/dev/null && echo OK || echo FAIL
        elif command -v curl &>/dev/null; then
            curl -sL '$TS_URL' -o '$TS_TARBALL' 2>/dev/null && echo OK || echo FAIL
        else
            echo NO_DOWNLOADER
        fi
    " 2>/dev/null || echo "FAIL")

    case "$DOWNLOAD_RESULT" in
        *OK*)
            echo "  Downloaded on tablet."
            ;;
        *)
            echo "  Downloading on local machine..."
            curl -sL "$TS_URL" -o "/tmp/$TS_TARBALL" || {
                echo "  ERROR: Failed to download Tailscale from $TS_URL"
                return 1
            }
            echo "  Uploading to tablet..."
            scp "${SSH_OPTS[@]}" "/tmp/$TS_TARBALL" "root@${TABLET_IP}:/tmp/$TS_TARBALL" || {
                echo "  ERROR: Failed to upload to tablet."
                return 1
            }
            rm -f "/tmp/$TS_TARBALL"
            ;;
    esac

    # Extract and install
    echo "  Installing on tablet..."
    run_remote "
        cd /tmp
        tar xzf '$TS_TARBALL'
        cp tailscale_${TS_VERSION}_${TS_ARCH}/tailscale /usr/local/bin/tailscale
        cp tailscale_${TS_VERSION}_${TS_ARCH}/tailscaled /usr/local/bin/tailscaled
        chmod +x /usr/local/bin/tailscale /usr/local/bin/tailscaled
        rm -rf tailscale_${TS_VERSION}_${TS_ARCH} '$TS_TARBALL'
        mkdir -p /var/lib/tailscale
    "
    echo "  Binaries installed to /usr/local/bin/"

    # Deploy systemd service
    ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "cat > /etc/systemd/system/tailscaled.service" <<'TSSERVICE'
[Unit]
Description=Tailscale node agent
After=network-online.target
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/local/bin/tailscaled --state=/var/lib/tailscale/tailscaled.state --socket=/var/run/tailscale/tailscaled.sock
ExecStopPost=/usr/local/bin/tailscale down
Restart=on-failure
RuntimeDirectory=tailscale
StateDirectory=tailscale

[Install]
WantedBy=multi-user.target
TSSERVICE

    run_remote 'systemctl daemon-reload && systemctl enable tailscaled && systemctl start tailscaled' 2>/dev/null || true

    # Wait briefly for tailscaled to start
    sleep 2

    echo ""
    echo "  Tailscale is installed and running."
    echo ""

    # Auth
    echo "  How would you like to authenticate?"
    echo "    1) Auth key (pre-generated at https://login.tailscale.com/admin/settings/keys)"
    echo "    2) Login URL (open on your phone/computer)"
    echo ""
    read -r -p "  Choose [1/2]: " AUTH_CHOICE

    case "$AUTH_CHOICE" in
        1)
            read -r -p "  Paste your auth key: " TS_AUTHKEY
            if [ -n "$TS_AUTHKEY" ]; then
                run_remote "tailscale up --authkey='$TS_AUTHKEY'" 2>&1 | sed 's/^/  /'
                echo ""
                echo "  Tailscale connected!"
                TS_IP=$(run_remote 'tailscale ip -4 2>/dev/null || echo "unknown"' | tr -d '[:space:]')
                echo "  Your tablet's Tailscale IP: $TS_IP"
                echo "  You can now SSH from anywhere: ssh root@$TS_IP"
            else
                echo "  No key provided. Run manually on the tablet:"
                echo "    tailscale up --authkey=tskey-auth-..."
            fi
            ;;
        *)
            echo "  Starting Tailscale login..."
            echo "  A URL will appear below — open it on your phone or computer."
            echo ""
            # tailscale up prints the login URL to stderr
            run_remote 'tailscale up' 2>&1 | sed 's/^/  /'
            echo ""
            TS_IP=$(run_remote 'tailscale ip -4 2>/dev/null || echo "pending"' | tr -d '[:space:]')
            if [ "$TS_IP" != "pending" ]; then
                echo "  Tailscale connected!"
                echo "  Your tablet's Tailscale IP: $TS_IP"
                echo "  You can now SSH from anywhere: ssh root@$TS_IP"
            else
                echo "  Complete the login at the URL above."
                echo "  Then check: ssh root@$TABLET_IP 'tailscale ip -4'"
            fi
            ;;
    esac
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

# ---- Step 6: Deploy services and configure boot mode ----
echo ""
echo "[6/6] Setting up boot mode..."
echo ""

# Deploy the quick-launch script
ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "cat > $REMOTE_LAUNCHER && chmod +x $REMOTE_LAUNCHER" <<'LAUNCHER_EOF'
#!/bin/sh
# Launch remarkable-ssh.
# Run:  ./term                          (local shell)
#       ./term user@server              (SSH to a server)
#       ./term --keyboard user@server   (with on-screen keyboard)
exec /home/root/remarkable-ssh "$@"
LAUNCHER_EOF
echo "  Quick-launch script deployed: $REMOTE_LAUNCHER"

# Deploy the terminal-only systemd service (direct boot to terminal, no menu)
ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "cat > /etc/systemd/system/remarkable-ssh.service" <<'SERVICE_EOF'
[Unit]
Description=remarkable-ssh terminal
After=basic.target

[Service]
Type=simple
ExecStart=/home/root/remarkable-ssh
ExecStopPost=/bin/sh -c 'systemctl start xochitl 2>/dev/null || true'
Restart=no
StandardInput=null
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
SERVICE_EOF
echo "  Terminal service deployed: remarkable-ssh.service"

# Deploy the launcher systemd service (menu: Terminal / Reader)
ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "cat > /etc/systemd/system/remarkable-launcher.service" <<'LAUNCHSVC_EOF'
[Unit]
Description=remarkable-ssh boot menu
After=xochitl.service
Wants=xochitl.service

[Service]
Type=simple
# Wait for rm2fb shared memory to appear before starting
ExecStartPre=/bin/sh -c 'for i in 1 2 3 4 5 6; do test -e /dev/shm/swtfb.01 && exit 0; sleep 2; done; exit 0'
ExecStart=/home/root/remarkable-ssh --launcher
Restart=on-failure
RestartSec=2
StandardInput=null
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
LAUNCHSVC_EOF
echo "  Launcher service deployed: remarkable-launcher.service"

# Deploy the mode toggle script
ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "cat > /home/root/term-mode && chmod +x /home/root/term-mode" <<'TOGGLE_EOF'
#!/bin/sh
# Switch boot mode: launcher (menu), terminal (direct), or reader (default).
#
# Usage:
#   ./term-mode launcher    Boot to menu (choose Terminal or Reader)
#   ./term-mode terminal    Boot directly to terminal
#   ./term-mode reader      Boot to e-reader (default reMarkable behavior)
#   ./term-mode             Show current mode

disable_all() {
    systemctl disable remarkable-launcher 2>/dev/null || true
    systemctl disable remarkable-ssh 2>/dev/null || true
}

case "${1:-}" in
    launcher|menu)
        disable_all
        systemctl enable xochitl 2>/dev/null || true
        systemctl enable remarkable-launcher 2>/dev/null
        echo "Boot mode: LAUNCHER (choose Terminal or Reader at boot)"
        echo "Reboot to apply, or: systemctl start remarkable-launcher"
        ;;
    terminal|term)
        disable_all
        systemctl disable xochitl 2>/dev/null || true
        systemctl enable remarkable-ssh 2>/dev/null
        echo "Boot mode: TERMINAL (direct boot to shell)"
        echo "Reboot to apply, or: systemctl stop xochitl && systemctl start remarkable-ssh"
        ;;
    reader|off)
        disable_all
        systemctl enable xochitl 2>/dev/null || true
        echo "Boot mode: READER (normal e-reader)"
        echo "Reboot to apply, or: systemctl start xochitl"
        ;;
    *)
        if systemctl is-enabled remarkable-launcher &>/dev/null; then
            echo "Current mode: LAUNCHER (menu at boot)"
        elif systemctl is-enabled remarkable-ssh &>/dev/null; then
            echo "Current mode: TERMINAL (direct boot to shell)"
        else
            echo "Current mode: READER (normal e-reader)"
        fi
        echo ""
        echo "Available modes:"
        echo "  ./term-mode launcher   Boot to menu (Terminal / Reader chooser)"
        echo "  ./term-mode terminal   Boot directly to terminal"
        echo "  ./term-mode reader     Back to normal e-reader"
        ;;
esac
TOGGLE_EOF
echo "  Mode toggle deployed: /home/root/term-mode"

# Ask user which boot mode they want
echo ""
echo "  How should the tablet boot?"
echo "    1) Launcher menu - choose Terminal or Reader each boot (recommended)"
echo "    2) Terminal       - boot straight to terminal"
echo "    3) Reader         - keep normal e-reader boot (launch terminal manually)"
echo ""
read -r -p "  Choose [1/2/3]: " BOOT_CHOICE

run_remote 'systemctl daemon-reload'

case "$BOOT_CHOICE" in
    2)
        run_remote '
            systemctl disable remarkable-launcher 2>/dev/null || true
            systemctl disable xochitl 2>/dev/null || true
            systemctl enable remarkable-ssh 2>/dev/null
        ' || true
        echo "  Boot mode: TERMINAL (direct boot to shell)"
        ;;
    3)
        run_remote '
            systemctl disable remarkable-launcher 2>/dev/null || true
            systemctl disable remarkable-ssh 2>/dev/null || true
            systemctl enable xochitl 2>/dev/null
        ' || true
        echo "  Boot mode: READER (use ./term to start terminal manually)"
        ;;
    *)
        run_remote '
            systemctl disable remarkable-ssh 2>/dev/null || true
            systemctl enable xochitl 2>/dev/null
            systemctl enable remarkable-launcher 2>/dev/null
        ' || true
        echo "  Boot mode: LAUNCHER (Terminal / Reader menu at boot)"
        ;;
esac

echo ""
echo "  Change anytime: ssh root@$TABLET_IP ./term-mode <launcher|terminal|reader>"

# ---- Step 7 (Optional): Tailscale ----
echo ""
echo "=== Optional: Tailscale ==="
echo ""
echo "  Tailscale gives your tablet a stable IP reachable from anywhere"
echo "  on your private network — no port forwarding needed."
echo ""
read -r -p "  Install Tailscale? [y/N] " INSTALL_TS

if [[ "$INSTALL_TS" =~ ^[Yy] ]]; then
    install_tailscale
fi

echo ""
echo "=== Install Complete ==="
echo ""
echo "Your reMarkable is ready."
echo ""
echo "  Reboot the tablet to start."
echo ""
case "$BOOT_CHOICE" in
    2)
        echo "  You'll get a terminal — SSH into your server from there:"
        echo "    ssh user@your-server"
        echo ""
        echo "  When you exit the shell, the e-reader comes back."
        ;;
    3)
        echo "  To launch the terminal manually:"
        echo "    ssh root@$TABLET_IP ./term"
        ;;
    *)
        echo "  You'll see a menu to choose Terminal or Reader."
        echo "  Pick Terminal to get a shell, then SSH to your server:"
        echo "    ssh user@your-server"
        echo ""
        echo "  Pick Reader to use the e-reader normally."
        echo "  To get back to the menu from reader: reboot the tablet."
        ;;
esac
echo ""
echo "To switch boot modes:"
echo "  ssh root@$TABLET_IP ./term-mode launcher   # menu at boot"
echo "  ssh root@$TABLET_IP ./term-mode terminal   # straight to terminal"
echo "  ssh root@$TABLET_IP ./term-mode reader     # normal e-reader"
echo ""
echo "To undo everything:"
echo "  ssh root@$TABLET_IP './term-mode reader; systemctl disable remarkable-launcher; rm /etc/systemd/system/remarkable-ssh.service /etc/systemd/system/remarkable-launcher.service; systemctl daemon-reload; systemctl restart xochitl'"
