#!/usr/bin/env bash
#
# uninstall.sh — Remove all remarkable-ssh components from the tablet
#
# Usage:
#   ./scripts/uninstall.sh [TABLET_IP]
#
# This cleans up:
#   - Custom systemd services (remarkable-ssh, remarkable-launcher)
#   - xochitl LD_PRELOAD override
#   - Tailscale binaries and service
#   - rm2fb library
#   - remarkable-ssh binary and launcher script
#
set -euo pipefail

TABLET_IP="${1:-10.11.99.1}"
SSH_OPTS=(-o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)

run_remote() {
    ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "$@"
}

echo "=== remarkable-ssh Uninstall ==="
echo ""

echo "[preflight] Checking tablet connectivity..."
if ! run_remote 'echo ok' &>/dev/null; then
    echo "ERROR: Cannot reach tablet at $TABLET_IP"
    exit 1
fi

echo "[1/4] Disabling and removing custom services..."
run_remote '
    systemctl disable remarkable-ssh 2>/dev/null || true
    systemctl disable remarkable-launcher 2>/dev/null || true
    systemctl stop remarkable-ssh 2>/dev/null || true
    systemctl stop remarkable-launcher 2>/dev/null || true
    rm -f /etc/systemd/system/remarkable-ssh.service
    rm -f /etc/systemd/system/remarkable-launcher.service
    rm -rf /etc/systemd/system/xochitl.service.d
' 2>/dev/null || true
echo "  Done."

echo "[2/4] Removing Tailscale..."
run_remote '
    systemctl disable tailscaled 2>/dev/null || true
    systemctl stop tailscaled 2>/dev/null || true
    rm -f /etc/systemd/system/tailscaled.service
    # Remove from both possible locations (old: /usr/local/bin, new: /home/root/bin)
    rm -f /usr/local/bin/tailscale /usr/local/bin/tailscaled
    rm -f /home/root/bin/tailscale /home/root/bin/tailscaled
    rm -rf /var/lib/tailscale /home/root/.tailscale
' 2>/dev/null || true
echo "  Done."

echo "[3/4] Removing rm2fb and remarkable-ssh files..."
run_remote '
    rm -f /home/root/remarkable-ssh
    rm -f /home/root/term
    rm -f /home/root/term-mode
    rm -f /home/root/lib/librm2fb_server.so*
    rm -f /opt/lib/librm2fb_server.so* 2>/dev/null || true
    rm -f /etc/rm2fb.conf
' 2>/dev/null || true
echo "  Done."

echo "[4/4] Restoring default boot and reloading systemd..."
run_remote '
    systemctl enable xochitl 2>/dev/null || true
    systemctl daemon-reload
    systemctl restart xochitl 2>/dev/null || true
' 2>/dev/null || true
echo "  Done."

# Show disk space recovered
ROOT_FREE=$(run_remote 'df -h / | awk "NR==2{print \$4}"' 2>/dev/null || echo "unknown")
echo ""
echo "=== Uninstall Complete ==="
echo "  Root filesystem free: $ROOT_FREE"
echo "  Tablet restored to default e-reader mode."
echo ""
