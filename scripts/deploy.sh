#!/bin/bash
# Deploy remarkable-ssh to a reMarkable 2 tablet via SCP.
#
# USAGE:
#   ./scripts/deploy.sh [REMARKABLE_IP]
#
# Default IP is 10.11.99.1 (USB connection).
# You can also use the Wi-Fi IP shown in Settings > About.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
TARGET="armv7-unknown-linux-musleabihf"
BINARY_NAME="remarkable-ssh"
BINARY_PATH="${PROJECT_DIR}/target/${TARGET}/release/${BINARY_NAME}"

RM_IP="${1:-10.11.99.1}"
RM_USER="root"
RM_DEST="/home/root/${BINARY_NAME}"

if [ ! -f "$BINARY_PATH" ]; then
    echo "Binary not found at: $BINARY_PATH"
    echo "Run ./scripts/build.sh first."
    exit 1
fi

echo "Deploying to reMarkable at ${RM_IP}..."
echo "Binary: ${BINARY_PATH}"
echo "  Size: $(ls -lh "$BINARY_PATH" | awk '{print $5}')"
echo ""

# Copy binary
scp "$BINARY_PATH" "${RM_USER}@${RM_IP}:${RM_DEST}"
echo "Copied to ${RM_DEST}"

# Make executable
ssh "${RM_USER}@${RM_IP}" "chmod +x ${RM_DEST}"
echo "Made executable."

echo ""
echo "=== Deployed successfully ==="
echo ""
echo "To run on the reMarkable:"
echo ""
echo "  1. SSH into your reMarkable:"
echo "     ssh root@${RM_IP}"
echo ""
echo "  2. Stop the UI (xochitl) to free the framebuffer:"
echo "     systemctl stop xochitl"
echo ""
echo "  3. Run the terminal:"
echo "     ${RM_DEST} user@your-server-ip"
echo ""
echo "  4. When done, restart the UI:"
echo "     systemctl start xochitl"
echo ""
echo "  Or, run it as a one-liner:"
echo "     systemctl stop xochitl && ${RM_DEST} user@100.64.0.1 ; systemctl start xochitl"
