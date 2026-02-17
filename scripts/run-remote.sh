#!/bin/bash
# One-shot: build, deploy, and run remarkable-ssh on the reMarkable.
#
# USAGE:
#   ./scripts/run-remote.sh [REMARKABLE_IP] [SSH_TARGET]
#
# EXAMPLES:
#   ./scripts/run-remote.sh                           # local shell on rM
#   ./scripts/run-remote.sh 10.11.99.1 user@100.64.0.1  # SSH to Tailscale host
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
TARGET="armv7-unknown-linux-musleabihf"
BINARY_NAME="remarkable-ssh"
BINARY_PATH="${PROJECT_DIR}/target/${TARGET}/release/${BINARY_NAME}"

RM_IP="${1:-10.11.99.1}"
SSH_TARGET="${2:-}"
RM_USER="root"
RM_DEST="/home/root/${BINARY_NAME}"

# Step 1: Build
echo "=== Building ==="
"$SCRIPT_DIR/build.sh"
echo ""

# Step 2: Deploy
echo "=== Deploying to ${RM_IP} ==="
scp "$BINARY_PATH" "${RM_USER}@${RM_IP}:${RM_DEST}"
ssh "${RM_USER}@${RM_IP}" "chmod +x ${RM_DEST}"
echo "Deployed."
echo ""

# Step 3: Run
echo "=== Running on reMarkable ==="
echo "Stopping xochitl..."

CMD="${RM_DEST}"
if [ -n "$SSH_TARGET" ]; then
    CMD="${CMD} ${SSH_TARGET}"
fi

# Stop xochitl, run our terminal, then restart xochitl
ssh -t "${RM_USER}@${RM_IP}" "systemctl stop xochitl && ${CMD} ; systemctl start xochitl"

echo ""
echo "Session ended. xochitl restarted."
