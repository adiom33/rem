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

# ---- Root partition space check ----
check_root_space() {
    local needed_kb="${1:-100}"
    local label="${2:-files}"
    local avail_kb
    avail_kb=$(run_remote 'df -k / | tail -1 | awk "{print \$4}"' 2>/dev/null || echo "")
    # Validate we got a number — BusyBox df or unexpected output could return garbage
    if ! echo "$avail_kb" | grep -qE '^[0-9]+$'; then
        echo "  WARNING: Could not parse root partition free space (got: '$avail_kb')."
        echo "  Assuming insufficient space for safety. Check manually:"
        echo "    ssh root@${TABLET_IP} 'df -h /'"
        return 1
    fi
    if [ "$avail_kb" -lt "$needed_kb" ]; then
        echo "  ERROR: Root partition has only ${avail_kb}KB free (need ${needed_kb}KB for ${label})."
        echo "  The root filesystem is tiny (256MB). Do NOT install large files there."
        echo "  Free space with: ssh root@${TABLET_IP} 'df -h /'"
        return 1
    fi
    echo "  Root partition: ${avail_kb}KB free (need ${needed_kb}KB for ${label})"
    return 0
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
    TS_VERSION=$(curl -s https://pkgs.tailscale.com/stable/ | sed -n 's/.*tailscale_\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\).*/\1/p' | sort -t. -k1,1n -k2,2n -k3,3n | tail -1 || echo "")
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

    # Extract and install — IMPORTANT: install to /home partition, NOT root partition!
    # The root filesystem is only 256MB and fills up easily.
    # Installing 60MB of Tailscale binaries to /usr/local/bin will kill SSH/dropbear.
    echo "  Installing on tablet (to /home/root/bin/ — safe for large binaries)..."
    run_remote "
        cd /tmp
        tar xzf '$TS_TARBALL'
        mkdir -p /home/root/bin
        cp tailscale_${TS_VERSION}_${TS_ARCH}/tailscale /home/root/bin/tailscale
        cp tailscale_${TS_VERSION}_${TS_ARCH}/tailscaled /home/root/bin/tailscaled
        chmod +x /home/root/bin/tailscale /home/root/bin/tailscaled
        rm -rf tailscale_${TS_VERSION}_${TS_ARCH} '$TS_TARBALL'
        mkdir -p /home/root/.local/share/tailscale
    "
    echo "  Binaries installed to /home/root/bin/"

    # Deploy systemd service
    ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "cat > /etc/systemd/system/tailscaled.service" <<'TSSERVICE'
[Unit]
Description=Tailscale node agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/home/root/bin/tailscaled --state=/home/root/.local/share/tailscale/tailscaled.state --socket=/var/run/tailscale/tailscaled.sock
ExecStopPost=/home/root/bin/tailscale down
Restart=on-failure
RuntimeDirectory=tailscale

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
                # Sanitize: auth keys should be alphanumeric + hyphens only
                CLEAN_KEY=$(echo "$TS_AUTHKEY" | tr -cd 'a-zA-Z0-9_-')
                run_remote "/home/root/bin/tailscale up --authkey='$CLEAN_KEY'" 2>&1 | sed 's/^/  /'
                echo ""
                echo "  Tailscale connected!"
                TS_IP=$(run_remote '/home/root/bin/tailscale ip -4 2>/dev/null || echo "unknown"' | tr -d '[:space:]')
                echo "  Your tablet's Tailscale IP: $TS_IP"
                echo "  You can now SSH from anywhere: ssh root@$TS_IP"
            else
                echo "  No key provided. Run manually on the tablet:"
                echo "    /home/root/bin/tailscale up --authkey=tskey-auth-..."
            fi
            ;;
        *)
            echo "  Starting Tailscale login..."
            echo "  A URL will appear below — open it on your phone or computer."
            echo "  (Times out after 120 seconds. Run '/home/root/bin/tailscale up' on the tablet to retry.)"
            echo ""
            run_remote '/home/root/bin/tailscale up --timeout=120s' 2>&1 | sed 's/^/  /'
            echo ""
            TS_IP=$(run_remote '/home/root/bin/tailscale ip -4 2>/dev/null || echo "pending"' | tr -d '[:space:]')
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

# Check root partition has enough space for systemd service files (~5KB)
echo ""
echo "[preflight] Checking root partition space..."
if ! check_root_space 500 "systemd service files"; then
    echo ""
    echo "  WARNING: Root partition is nearly full. Proceeding may brick the device."
    echo "  Free space before continuing."
    exit 1
fi
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
    # NOTE: We follow the GitHub "latest" redirect to get the actual filename
    # instead of hardcoding a version, since the .so version changes across releases.
    DOWNLOAD_OK=$(run_remote '
        # Ensure /opt/lib exists on /home partition, not on root.
        # After factory reset, /opt symlink may be broken (target wiped).
        if [ -L /opt ] && [ ! -d /opt ]; then
            # Broken symlink — recreate the target
            OPTDIR=$(readlink /opt 2>/dev/null || echo "/home/opt")
            mkdir -p "$OPTDIR/lib"
        elif [ ! -e /opt ]; then
            # /opt does not exist at all — create on /home and symlink
            mkdir -p /home/opt/lib
            ln -sf /home/opt /opt
        else
            mkdir -p /opt/lib
        fi
        cd /tmp
        rm -f librm2fb_server.so*

        # Download the latest release .so — follow redirects to get the actual file
        SO_NAME="librm2fb_server.so.1.0.1"
        DL_URL="https://github.com/ddvk/remarkable2-framebuffer/releases/latest/download/$SO_NAME"
        if command -v wget &>/dev/null; then
            wget -q "$DL_URL" -O "$SO_NAME" 2>/dev/null
        elif command -v curl &>/dev/null; then
            curl -sL "$DL_URL" -o "$SO_NAME" 2>/dev/null
        else
            echo "NO_DOWNLOADER"
            exit 1
        fi

        # Validate: the file must be an ELF binary, not an HTML error page
        if [ -f "$SO_NAME" ] && [ -s "$SO_NAME" ]; then
            MAGIC=$(head -c4 "$SO_NAME" | od -A n -t x1 | tr -d " " | head -1)
            if [ "$MAGIC" = "7f454c46" ]; then
                mv "$SO_NAME" /opt/lib/
                cd /opt/lib
                ln -sf "$SO_NAME" librm2fb_server.so.1
                echo "OK"
            else
                echo "NOT_ELF"
                rm -f "$SO_NAME"
            fi
        else
            echo "DOWNLOAD_FAILED"
        fi
    ' 2>/dev/null || echo "FAILED")

    case "$DOWNLOAD_OK" in
        *OK*)
            echo "  Downloaded and installed rm2fb server .so"
            HAS_SERVER="yes"
            ;;
        *NOT_ELF*)
            echo "  ERROR: Downloaded file is not a valid ELF binary (likely a 404 HTML page)."
            echo "  The rm2fb release may have changed filenames."
            echo "  Check https://github.com/ddvk/remarkable2-framebuffer/releases manually."
            echo "  Then: scp <file> root@${TABLET_IP}:/opt/lib/"
            ;;
        *NO_DOWNLOADER*)
            echo "  No wget/curl on tablet. Download manually:"
            echo "    Visit: https://github.com/ddvk/remarkable2-framebuffer/releases"
            echo "    scp librm2fb_server.so.* root@${TABLET_IP}:/opt/lib/"
            echo "    ssh root@${TABLET_IP} 'cd /opt/lib && ln -sf librm2fb_server.so.* librm2fb_server.so.1'"
            ;;
        *)
            echo "  Download failed (tablet may not have internet access)."
            echo "  Download on your computer and deploy:"
            echo "    Visit: https://github.com/ddvk/remarkable2-framebuffer/releases"
            echo "    scp librm2fb_server.so.* root@${TABLET_IP}:/opt/lib/"
            echo "    ssh root@${TABLET_IP} 'cd /opt/lib && ln -sf librm2fb_server.so.* librm2fb_server.so.1'"
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
        echo ""
        echo "  ERROR: /dev/shm/swtfb.01 not found after restart."
        echo "  rm2fb failed to start. ROLLING BACK the LD_PRELOAD override"
        echo "  to prevent xochitl crash-loops on reboot."
        echo ""
        run_remote '
            rm -f /etc/systemd/system/xochitl.service.d/rm2fb.conf
            rmdir /etc/systemd/system/xochitl.service.d 2>/dev/null
            systemctl daemon-reload
            systemctl restart xochitl
        ' 2>/dev/null || true
        echo "  Override removed. xochitl restarted without rm2fb."
        echo ""
        echo "  Common causes:"
        echo "  - 'Missing address for function': addresses in rm2fb.conf are wrong"
        echo "  - Crash/segfault: rm2fb .so is ABI-incompatible with this firmware"
        echo "  - Qt errors: rm2fb was built against a different Qt version"
        echo ""
        echo "  Debug with: ssh root@$TABLET_IP 'journalctl -u xochitl -n 30'"
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
After=multi-user.target
# Safety: if this service fails 3 times, re-enable xochitl so the device
# remains accessible via SSH (USB networking depends on xochitl's boot chain).

[Service]
Type=simple
ExecStart=/home/root/remarkable-ssh
# On any exit (clean or crash), re-enable and start xochitl so USB networking
# stays up both now AND after reboot. Without this, hitting StartLimitBurst
# leaves xochitl disabled and the device unreachable after the next reboot.
ExecStopPost=/bin/sh -c 'systemctl enable xochitl 2>/dev/null; systemctl start xochitl 2>/dev/null || true'
Restart=on-failure
RestartSec=3
StartLimitBurst=3
StartLimitIntervalSec=60
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
ExecStartPre=/bin/sh -c 'for i in 1 2 3 4 5 6; do test -e /dev/shm/swtfb.01 && exit 0; sleep 2; done; exit 1'
ExecStart=/home/root/remarkable-ssh --launcher
Restart=on-failure
RestartSec=5
# Give up after 5 failures in 3 minutes — fall back to xochitl
StartLimitBurst=5
StartLimitIntervalSec=180
ExecStopPost=/bin/sh -c 'systemctl start xochitl 2>/dev/null || true'
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
echo "  ssh root@$TABLET_IP './term-mode reader; systemctl disable remarkable-launcher tailscaled 2>/dev/null; rm -f /etc/systemd/system/remarkable-ssh.service /etc/systemd/system/remarkable-launcher.service /etc/systemd/system/tailscaled.service /etc/systemd/system/xochitl.service.d/rm2fb.conf /etc/rm2fb.conf; rmdir /etc/systemd/system/xochitl.service.d 2>/dev/null; systemctl daemon-reload; systemctl restart xochitl'"
