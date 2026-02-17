#!/bin/bash
# Build remarkable-ssh for the reMarkable 2 tablet.
#
# The reMarkable 2 uses an ARM Cortex-A7 (armv7l) processor.
# We cross-compile a static binary using musl so it has zero runtime dependencies.
#
# BUILD METHODS (tried in order):
#   1. `cross` (recommended - uses Docker, no toolchain setup needed)
#   2. arm-linux-musleabihf-gcc → musl target (static binary, preferred)
#   3. arm-linux-gnueabihf-gcc → gnueabihf target (dynamic, works if glibc on device)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BINARY_NAME="remarkable-ssh"

# Preflight: check for cargo/rustup
for tool in cargo rustup; do
    if ! command -v "$tool" &>/dev/null; then
        echo "ERROR: '$tool' not found. Install Rust first:"
        echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        exit 1
    fi
done

cd "$PROJECT_DIR"

# ---- Method 1: Try `cross` first (Docker-based, most reliable) ----
if command -v cross &>/dev/null; then
    TARGET="armv7-unknown-linux-musleabihf"
    echo "Building with 'cross' (Docker-based cross-compilation)..."
    cross build --release --target "$TARGET"
    echo ""
    echo "Build successful!"
    echo "Binary: target/${TARGET}/release/${BINARY_NAME}"
    ls -lh "target/${TARGET}/release/${BINARY_NAME}"
    exit 0
fi

# ---- Method 2+3: Native cross-compilation ----
echo "'cross' not found. Trying native cross-compilation..."
echo ""

if command -v arm-linux-musleabihf-gcc &>/dev/null; then
    # Method 2: musl linker → musl target (static binary, ideal)
    TARGET="armv7-unknown-linux-musleabihf"
    echo "Using linker: arm-linux-musleabihf-gcc (static musl binary)"
elif command -v arm-linux-gnueabihf-gcc &>/dev/null; then
    # Method 3: gnueabihf linker → gnueabihf target (correct pairing)
    TARGET="armv7-unknown-linux-gnueabihf"
    echo "Using linker: arm-linux-gnueabihf-gcc"
    echo "NOTE: Building dynamically-linked binary (gnueabihf target)."
    echo "  For a static binary, install 'cross': cargo install cross"
else
    echo ""
    echo "ERROR: No ARM cross-linker found."
    echo ""
    echo "Install one of these:"
    echo ""
    echo "  # Easiest (uses Docker, works everywhere):"
    echo "  cargo install cross"
    echo ""
    echo "  # Ubuntu/Debian (static musl binary, preferred):"
    echo "  sudo apt install musl-tools"
    echo ""
    echo "  # Ubuntu/Debian (dynamic gnueabihf binary):"
    echo "  sudo apt install gcc-arm-linux-gnueabihf"
    echo ""
    echo "  # macOS (via Homebrew):"
    echo "  brew install filosottile/musl-cross/musl-cross"
    echo ""
    exit 1
fi

# Ensure the Rust target is installed
if ! rustup target list --installed 2>/dev/null | grep -q "$TARGET"; then
    echo "Adding Rust target: $TARGET"
    rustup target add "$TARGET"
fi

echo "Building with cargo for target: $TARGET"
cargo build --release --target "$TARGET"

echo ""
echo "Build successful!"
BINARY_PATH="target/${TARGET}/release/${BINARY_NAME}"
echo "Binary: ${BINARY_PATH}"
ls -lh "$BINARY_PATH"

# Verify it's an ARM binary
if command -v file &>/dev/null; then
    FILE_OUT="$(file "$BINARY_PATH")"
    if echo "$FILE_OUT" | grep -q "ARM"; then
        echo "Verified: ARM binary"
    else
        echo "WARNING: Binary may not be ARM: $FILE_OUT"
    fi
fi
