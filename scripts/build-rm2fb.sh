#!/usr/bin/env bash
#
# build-rm2fb.sh — Build rm2fb from source for your reMarkable firmware
#
# This script:
#   1. Clones the rm2fb repo
#   2. Pulls Qt headers + xochitl from the tablet (uses the tablet's own libraries)
#   3. Extracts function addresses from xochitl
#   4. Builds librm2fb_server.so targeting your firmware
#   5. Deploys everything to the tablet
#
# Usage:
#   ./scripts/build-rm2fb.sh [TABLET_IP]
#
# Prerequisites:
#   - ARM cross-compiler (arm-linux-gnueabihf-g++ or similar)
#   - SSH access to the tablet
#   - git
#
set -euo pipefail

TABLET_IP="${1:-10.11.99.1}"
SSH_OPTS=(-o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)
WORK_DIR="$(pwd)/rm2fb-build"
RM2FB_REPO="https://github.com/ddvk/remarkable2-framebuffer.git"

run_remote() {
    ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "$@"
}

echo "=== rm2fb Source Build ==="
echo ""

# ---- Preflight ----
echo "[preflight] Checking prerequisites..."

# Check for ARM C++ compiler
CXX=""
for candidate in arm-linux-gnueabihf-g++ arm-remarkable-linux-gnueabi-g++ arm-none-linux-gnueabihf-g++; do
    if command -v "$candidate" &>/dev/null; then
        CXX="$candidate"
        break
    fi
done
if [ -z "$CXX" ]; then
    echo "ERROR: No ARM C++ cross-compiler found."
    echo ""
    echo "Install one of:"
    echo "  Ubuntu/Debian: sudo apt install g++-arm-linux-gnueabihf"
    echo "  Fedora:        sudo dnf install arm-linux-gnueabihf-gcc-c++"
    echo "  macOS:         brew install arm-linux-gnueabihf-binutils"
    echo "                 (or use the reMarkable toolchain)"
    echo ""
    echo "Or use the reMarkable SDK toolchain:"
    echo "  https://remarkable.guide/devel/toolchains.html"
    exit 1
fi
echo "  C++ compiler: $CXX"

# Check for git
if ! command -v git &>/dev/null; then
    echo "ERROR: git not found"
    exit 1
fi

# Check tablet connectivity
if ! run_remote 'echo ok' &>/dev/null; then
    echo "ERROR: Cannot reach tablet at $TABLET_IP"
    echo "  Make sure the tablet is connected via USB and SSH works:"
    echo "  ssh root@$TABLET_IP"
    exit 1
fi

FIRMWARE_VER=$(run_remote 'grep REMARKABLE_RELEASE_VERSION /usr/share/remarkable/update.conf 2>/dev/null | cut -d= -f2 || cat /etc/version 2>/dev/null || echo unknown')
FIRMWARE_VER=$(echo "$FIRMWARE_VER" | tr -d '[:space:]')
echo "  Tablet firmware: $FIRMWARE_VER"
echo ""

# ---- Step 1: Clone rm2fb ----
echo "[1/6] Cloning rm2fb..."
mkdir -p "$WORK_DIR"
if [ -d "$WORK_DIR/remarkable2-framebuffer" ]; then
    echo "  Already cloned, pulling latest..."
    cd "$WORK_DIR/remarkable2-framebuffer"
    git pull --ff-only 2>/dev/null || true
    cd - >/dev/null
else
    git clone "$RM2FB_REPO" "$WORK_DIR/remarkable2-framebuffer"
fi
RM2FB_SRC="$WORK_DIR/remarkable2-framebuffer"
echo ""

# ---- Step 2: Pull Qt headers from the tablet ----
echo "[2/6] Pulling Qt headers and libraries from tablet..."
SYSROOT="$WORK_DIR/sysroot"
mkdir -p "$SYSROOT/usr/include" "$SYSROOT/usr/lib"

# Pull Qt headers
run_remote 'tar czf - /usr/include/qt5 2>/dev/null || tar czf - /usr/include/Qt* 2>/dev/null || echo "NO_QT_HEADERS"' > "$WORK_DIR/qt-headers.tar.gz" || true
if [ -s "$WORK_DIR/qt-headers.tar.gz" ] && ! grep -q "NO_QT_HEADERS" "$WORK_DIR/qt-headers.tar.gz" 2>/dev/null; then
    tar xzf "$WORK_DIR/qt-headers.tar.gz" -C "$SYSROOT" 2>/dev/null || true
    echo "  Qt headers extracted"
else
    echo "  WARNING: No Qt headers on tablet (this is normal — reMarkable ships without dev headers)"
    echo "  Will try to use rm2fb's bundled headers..."

    # rm2fb may have its own minimal headers — check
    if [ -d "$RM2FB_SRC/src" ]; then
        echo "  Using rm2fb source headers"
    fi
fi

# Pull Qt libraries (needed for linking)
echo "  Pulling Qt shared libraries..."
run_remote 'tar czf - /usr/lib/libQt5Core.so* /usr/lib/libQt5Gui.so* /usr/lib/libQt5Widgets.so* /usr/lib/libQt5DBus.so* /usr/lib/libQt5Network.so* 2>/dev/null' > "$WORK_DIR/qt-libs.tar.gz" || true
if [ -s "$WORK_DIR/qt-libs.tar.gz" ]; then
    tar xzf "$WORK_DIR/qt-libs.tar.gz" -C "$SYSROOT" 2>/dev/null || true
    echo "  Qt libraries extracted"
    ls "$SYSROOT/usr/lib/libQt5"*.so* 2>/dev/null | head -5 | sed "s/^/    /"
else
    echo "  WARNING: Could not pull Qt libraries"
fi

# Pull other needed system libraries
echo "  Pulling system libraries..."
run_remote 'tar czf - /usr/lib/libstdc++.so* /usr/lib/libgcc_s.so* /lib/libc.so* /lib/libpthread.so* /lib/libdl.so* /lib/libm.so* /lib/librt.so* 2>/dev/null' > "$WORK_DIR/sys-libs.tar.gz" || true
if [ -s "$WORK_DIR/sys-libs.tar.gz" ]; then
    tar xzf "$WORK_DIR/sys-libs.tar.gz" -C "$SYSROOT" 2>/dev/null || true
fi
echo ""

# ---- Step 3: Pull xochitl and find addresses ----
echo "[3/6] Extracting function addresses from xochitl..."
XOCHITL="$WORK_DIR/xochitl"
scp "${SSH_OPTS[@]}" "root@${TABLET_IP}:/usr/bin/xochitl" "$XOCHITL"

echo "  Address extraction is handled by 'remarkable-ssh --setup' on the tablet."
echo "  It parses the xochitl ELF binary directly (no Ghidra/objdump needed)."
echo "  The install script runs this automatically."
echo ""
echo "  If you need to extract addresses manually from this xochitl copy,"
echo "  deploy it to the tablet and run: remarkable-ssh --setup"

# ---- Step 4: Build rm2fb ----
echo ""
echo "[4/6] Building rm2fb..."
cd "$RM2FB_SRC"

# Determine include paths
QT_INC=""
if [ -d "$SYSROOT/usr/include/qt5" ]; then
    QT_INC="$SYSROOT/usr/include/qt5"
elif [ -d "$SYSROOT/usr/include/QtCore" ]; then
    QT_INC="$SYSROOT/usr/include"
fi

QT_LIB="$SYSROOT/usr/lib"

# Build the server .so
# rm2fb uses qmake, but we can compile manually since it's small
echo "  Compiling rm2fb server..."

SERVER_SRCS=$(find src/server -name '*.cpp' 2>/dev/null || echo "")
SHARED_SRCS=$(find src/shared -name '*.cpp' 2>/dev/null || echo "")

if [ -z "$SERVER_SRCS" ]; then
    echo "  ERROR: No server source files found in $RM2FB_SRC/src/server/"
    echo "  The rm2fb repo structure may have changed."
    echo ""
    echo "  Listing available sources:"
    find src -name '*.cpp' 2>/dev/null | sed "s/^/    /"
    exit 1
fi

# Collect all include dirs
INC_FLAGS="-I src -I src/shared -I src/server"
if [ -n "$QT_INC" ]; then
    INC_FLAGS="$INC_FLAGS -I $QT_INC -I $QT_INC/QtCore -I $QT_INC/QtGui -I $QT_INC/QtWidgets"
fi

# Lib flags
LIB_FLAGS="-L $QT_LIB -lQt5Core -lQt5Gui"

echo "  Sources: $SERVER_SRCS $SHARED_SRCS"
echo "  Includes: $INC_FLAGS"
echo ""

$CXX -shared -fPIC -o "$WORK_DIR/librm2fb_server.so.1.0.1" \
    $INC_FLAGS \
    $SERVER_SRCS $SHARED_SRCS \
    $LIB_FLAGS \
    -std=c++17 \
    -ldl \
    --sysroot="$SYSROOT" \
    -Wl,-soname,librm2fb_server.so.1 \
    2>&1 | head -30 || {
    echo ""
    echo "  Build failed. Common fixes:"
    echo "  - Missing Qt headers: Install reMarkable SDK toolchain"
    echo "  - Wrong compiler: Try the reMarkable SDK's compiler"
    echo "  - ABI mismatch: The tablet's Qt version may need matching SDK"
    echo ""
    echo "  Alternative: Use the reMarkable Docker toolchain:"
    echo "    docker run --rm -v \$(pwd):/work -w /work/remarkable2-framebuffer \\"
    echo "      ghcr.io/toltec-dev/toolchain:latest \\"
    echo "      qmake && make"
    echo ""
    echo "  See: https://remarkable.guide/devel/toolchains.html"
    exit 1
}

echo "  Build succeeded!"
ls -la "$WORK_DIR/librm2fb_server.so.1.0.1"
echo ""

# ---- Step 5: Generate rm2fb.conf ----
echo "[5/6] Generating rm2fb.conf..."
echo ""
echo "  rm2fb.conf is generated on the tablet by 'remarkable-ssh --setup'."
echo "  After deploying the server .so (step 6), run the install script"
echo "  or manually run on the tablet: remarkable-ssh --setup"
echo ""
echo "  If auto-extraction fails, deploy addresses manually:"
echo "    ./scripts/deploy-rm2fb-conf.sh TABLET_IP 0x<update> 0x<create>"
echo ""

# ---- Step 6: Deploy ----
echo "[6/6] Ready to deploy"
echo ""
echo "  Built files:"
echo "    $WORK_DIR/librm2fb_server.so.1.0.1"
echo ""
echo "  Deploy the server .so:"
echo ""
echo "    scp $WORK_DIR/librm2fb_server.so.1.0.1 root@${TABLET_IP}:/opt/lib/"
echo "    ssh root@${TABLET_IP} 'cd /opt/lib && ln -sf librm2fb_server.so.1.0.1 librm2fb_server.so.1'"
echo ""
echo "  Then extract addresses and generate rm2fb.conf on the tablet:"
echo ""
echo "    ssh root@${TABLET_IP} '/home/root/remarkable-ssh --setup'"
echo ""
echo "  Test it:"
echo ""
echo "    ssh root@${TABLET_IP} 'systemctl stop xochitl && LD_PRELOAD=/opt/lib/librm2fb_server.so.1 xochitl &'"
echo ""
echo "  If xochitl starts and the UI appears, rm2fb is working."
echo "  Then run remarkable-ssh — it auto-detects rm2fb."
echo ""
echo "  === Docker alternative (recommended if the above build fails) ==="
echo ""
echo "  The Toltec toolchain Docker image has all the right Qt headers:"
echo ""
echo "    cd $RM2FB_SRC"
echo "    docker run --rm -v \$(pwd):/work -w /work \\"
echo "      ghcr.io/toltec-dev/toolchain:latest \\"
echo "      bash -c 'qmake && make'"
echo ""
