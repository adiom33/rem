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
BINARY_NAME="remarkable-ssh"

RM_IP="${1:-10.11.99.1}"
RM_USER="root"
RM_DEST="/home/root/${BINARY_NAME}"

SSH_OPTS=(-o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=3)

# Preflight checks
for tool in ssh scp; do
    if ! command -v "$tool" &>/dev/null; then
        echo "ERROR: '$tool' not found. Install OpenSSH."
        exit 1
    fi
done

# Find the built binary (build.sh may use musl or gnueabihf target)
BINARY_PATH=""
for target in armv7-unknown-linux-musleabihf armv7-unknown-linux-gnueabihf; do
    candidate="${PROJECT_DIR}/target/${target}/release/${BINARY_NAME}"
    if [ -f "$candidate" ]; then
        BINARY_PATH="$candidate"
        break
    fi
done

if [ -z "$BINARY_PATH" ]; then
    echo "Binary not found. Run ./scripts/build.sh first."
    exit 1
fi

# Get binary size (stat is more robust than ls+awk)
if stat --version &>/dev/null 2>&1; then
    # GNU stat
    BINARY_SIZE="$(stat -c %s "$BINARY_PATH")"
else
    # BSD/macOS stat
    BINARY_SIZE="$(stat -f %z "$BINARY_PATH")"
fi
BINARY_SIZE_KB=$((BINARY_SIZE / 1024))

echo "Deploying to reMarkable at ${RM_IP}..."
echo "Binary: ${BINARY_PATH}"
echo "  Size: ${BINARY_SIZE_KB}KB"
echo ""

# Copy binary
scp "${SSH_OPTS[@]}" -- "$BINARY_PATH" "${RM_USER}@${RM_IP}:${RM_DEST}"
echo "Copied to ${RM_DEST}"

# Make executable
ssh "${SSH_OPTS[@]}" -- "${RM_USER}@${RM_IP}" "chmod +x ${RM_DEST}"
echo "Made executable."

echo ""
echo "=== Deployed successfully ==="
echo ""
echo "To run on the reMarkable:"
echo ""
echo "  ssh root@${RM_IP}"
echo "  ${RM_DEST} user@your-server-ip"
echo ""
echo "Or use the all-in-one script:"
echo "  ./scripts/run-remote.sh ${RM_IP} user@your-server-ip"
