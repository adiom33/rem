#!/bin/bash
# Build remarkable-ssh for the reMarkable 2 tablet.
#
# The reMarkable 2 uses an ARM Cortex-A7 (armv7l) processor.
# We cross-compile a static binary using musl so it has zero runtime dependencies.
#
# THREE BUILD METHODS (tried in order):
#   1. Using `cross` (recommended - uses Docker, no toolchain setup needed)
#   2. Using arm-linux-musleabihf-gcc (musl cross-linker)
#   3. Using arm-linux-gnueabihf-gcc (glibc cross-linker, with musl target)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
TARGET="armv7-unknown-linux-musleabihf"
BINARY_NAME="remarkable-ssh"

cd "$PROJECT_DIR"

# ---- Method 1: Try `cross` first (Docker-based, most reliable) ----
if command -v cross &>/dev/null; then
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

# Check for the Rust target
if ! rustup target list --installed 2>/dev/null | grep -q "$TARGET"; then
    echo "Adding Rust target: $TARGET"
    rustup target add "$TARGET"
fi

# Try musl linker first, then gnueabihf as fallback
if command -v arm-linux-musleabihf-gcc &>/dev/null; then
    echo "Using linker: arm-linux-musleabihf-gcc"
    echo "Building with cargo for target: $TARGET"
    cargo build --release --target "$TARGET"
elif command -v arm-linux-gnueabihf-gcc &>/dev/null; then
    echo "Using linker: arm-linux-gnueabihf-gcc (gnueabihf fallback for musl target)"
    echo "Building with cargo for target: $TARGET"
    CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_LINKER=arm-linux-gnueabihf-gcc \
        cargo build --release --target "$TARGET"
else
    echo ""
    echo "ERROR: No ARM cross-linker found."
    echo ""
    echo "Install one of these:"
    echo ""
    echo "  # Easiest (uses Docker, works everywhere):"
    echo "  cargo install cross"
    echo ""
    echo "  # Ubuntu/Debian (musl):"
    echo "  sudo apt install musl-tools gcc-arm-linux-gnueabihf"
    echo ""
    echo "  # Ubuntu/Debian (gnueabihf only - also works):"
    echo "  sudo apt install gcc-arm-linux-gnueabihf"
    echo ""
    echo "  # macOS (via Homebrew):"
    echo "  brew install filosottile/musl-cross/musl-cross"
    echo ""
    exit 1
fi

echo ""
echo "Build successful!"
echo "Binary: target/${TARGET}/release/${BINARY_NAME}"
ls -lh "target/${TARGET}/release/${BINARY_NAME}"
