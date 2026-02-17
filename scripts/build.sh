#!/bin/bash
# Build remarkable-ssh for the reMarkable 2 tablet.
#
# The reMarkable 2 uses an ARM Cortex-A7 (armv7l) processor.
# We cross-compile a static binary using musl so it has zero runtime dependencies.
#
# TWO BUILD METHODS:
#   1. Using `cross` (recommended - uses Docker, no toolchain setup needed)
#   2. Using a manually installed ARM cross-compiler
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
TARGET="armv7-unknown-linux-musleabihf"
BINARY_NAME="remarkable-ssh"

cd "$PROJECT_DIR"

# ---- Method 1: Try `cross` first ----
if command -v cross &>/dev/null; then
    echo "Building with 'cross' (Docker-based cross-compilation)..."
    cross build --release --target "$TARGET"
    echo ""
    echo "Build successful!"
    echo "Binary: target/${TARGET}/release/${BINARY_NAME}"
    ls -lh "target/${TARGET}/release/${BINARY_NAME}"
    exit 0
fi

# ---- Method 2: Native cross-compilation ----
echo "'cross' not found. Trying native cross-compilation..."
echo ""

# Check for the Rust target
if ! rustup target list --installed | grep -q "$TARGET"; then
    echo "Adding Rust target: $TARGET"
    rustup target add "$TARGET"
fi

# Check for the cross-linker
if ! command -v arm-linux-musleabihf-gcc &>/dev/null; then
    echo ""
    echo "ERROR: Cross-linker not found: arm-linux-musleabihf-gcc"
    echo ""
    echo "Install it via one of these methods:"
    echo ""
    echo "  # Ubuntu/Debian:"
    echo "  sudo apt install gcc-arm-linux-gnueabihf musl-tools"
    echo ""
    echo "  # macOS (via Homebrew):"
    echo "  brew install filosottile/musl-cross/musl-cross"
    echo ""
    echo "  # Or install 'cross' for Docker-based builds (easiest):"
    echo "  cargo install cross"
    echo ""
    exit 1
fi

echo "Building with cargo for target: $TARGET"
cargo build --release --target "$TARGET"

echo ""
echo "Build successful!"
echo "Binary: target/${TARGET}/release/${BINARY_NAME}"
ls -lh "target/${TARGET}/release/${BINARY_NAME}"
