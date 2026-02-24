#!/bin/bash
# One-shot: build, deploy, and run remarkable-ssh on the reMarkable.
#
# USAGE:
#   ./scripts/run-remote.sh [REMARKABLE_IP] [SSH_TARGET] [EXTRA_ARGS...]
#
# EXAMPLES:
#   ./scripts/run-remote.sh                                    # local shell on rM
#   ./scripts/run-remote.sh 10.11.99.1 user@100.64.0.1        # SSH to Tailscale host
#   ./scripts/run-remote.sh 10.11.99.1 user@host --tmux       # with tmux
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BINARY_NAME="remarkable-ssh"

RM_IP="${1:-10.11.99.1}"
SSH_TARGET="${2:-}"
shift 2 2>/dev/null || true
EXTRA_ARGS=("$@")

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

# Step 1: Build
echo "=== Building ==="
"$SCRIPT_DIR/build.sh"

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
    echo "ERROR: Built binary not found. Check build output above."
    exit 1
fi
echo ""

# Step 2: Deploy
echo "=== Deploying to ${RM_IP} ==="
scp "${SSH_OPTS[@]}" -- "$BINARY_PATH" "${RM_USER}@${RM_IP}:${RM_DEST}"
ssh "${SSH_OPTS[@]}" -- "${RM_USER}@${RM_IP}" "chmod +x ${RM_DEST}"
echo "Deployed."
echo ""

# Step 3: Run
echo "=== Running on reMarkable ==="
echo ""
echo "TIP: If the screen stays blank, check the log output."
echo "     'No working MXCFB ioctl' means you need rm2fb. See README.md."
echo ""

# Build the remote command with proper quoting.
# The binary itself decides whether to stop xochitl based on the display backend:
#   - rm2fb backend: xochitl must stay running (rm2fb lives alongside it)
#   - native/other: xochitl must be stopped so we own the framebuffer
# So we let the binary manage xochitl lifecycle and just run it directly.
REMOTE_CMD="${RM_DEST}"

# Append SSH target as a properly quoted argument
if [ -n "$SSH_TARGET" ]; then
    # Use printf %q to safely shell-quote the argument
    REMOTE_CMD="${REMOTE_CMD} $(printf '%q' "$SSH_TARGET")"
fi

# Append any extra args (--tmux, --keyboard, etc.)
for arg in "${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}"; do
    REMOTE_CMD="${REMOTE_CMD} $(printf '%q' "$arg")"
done

# Safety net: if the binary crashes (SIGSEGV, SIGKILL, etc.), the Rust panic hook
# won't run and xochitl stays stopped — leaving a blank screen.  Wrap in a shell
# snippet that restarts xochitl on non-zero exit.
ssh -t "${SSH_OPTS[@]}" -- "${RM_USER}@${RM_IP}" "
    ${REMOTE_CMD}; rc=\$?;
    if [ \$rc -ne 0 ]; then
        systemctl start xochitl 2>/dev/null;
    fi;
    exit \$rc
"

echo ""
echo "Session ended."
