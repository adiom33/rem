#!/usr/bin/env bash
#
# rm2fb-check.sh — Check rm2fb compatibility on the reMarkable tablet
#
# Diagnoses whether a pre-built rm2fb .so will work or if you need to
# build from source. Also extracts xochitl marker strings for address finding.
#
# Usage:
#   ./scripts/rm2fb-check.sh [TABLET_IP]
#
set -euo pipefail

TABLET_IP="${1:-10.11.99.1}"
SSH_OPTS=(-o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)

run_remote() {
    ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" "$@"
}

echo "=== rm2fb Compatibility Check ==="
echo ""

# ---- 1. Firmware & device info ----
echo "[1/5] Device info"
run_remote '
    echo "  Model:    $(cat /sys/firmware/devicetree/base/model 2>/dev/null | tr -d "\0" || echo "unknown")"
    FW=$(grep REMARKABLE_RELEASE_VERSION /usr/share/remarkable/update.conf 2>/dev/null | cut -d= -f2 || cat /etc/version 2>/dev/null || echo "unknown")
    echo "  Firmware: $FW"
    echo "  Kernel:   $(uname -r)"
    echo "  Arch:     $(uname -m)"
'
echo ""

# ---- 2. Framebuffer driver ----
echo "[2/5] Framebuffer driver"
run_remote '
    if [ -e /sys/class/graphics/fb0 ]; then
        DRIVER=$(readlink -f /sys/class/graphics/fb0/device/driver 2>/dev/null | xargs basename 2>/dev/null || echo "unknown")
        echo "  fb0 driver: $DRIVER"
        cat /sys/class/graphics/fb0/virtual_size 2>/dev/null | xargs -I{} echo "  fb0 size:   {}" || true
        cat /sys/class/graphics/fb0/bits_per_pixel 2>/dev/null | xargs -I{} echo "  fb0 bpp:    {}" || true
    else
        echo "  /dev/fb0 not found!"
    fi
    echo ""
    echo "  /dev/shm/swtfb.01 exists: $([ -e /dev/shm/swtfb.01 ] && echo YES || echo NO)"
'
echo ""

# ---- 3. Qt libraries on the tablet ----
echo "[3/5] Qt libraries on tablet"
run_remote '
    echo "  Qt5Core:"
    ls -la /usr/lib/libQt5Core.so* 2>/dev/null | sed "s/^/    /" || echo "    NOT FOUND"
    echo "  Qt5Gui:"
    ls -la /usr/lib/libQt5Gui.so* 2>/dev/null | sed "s/^/    /" || echo "    NOT FOUND"
    echo "  Qt5Widgets:"
    ls -la /usr/lib/libQt5Widgets.so* 2>/dev/null | sed "s/^/    /" || echo "    NOT FOUND"
    echo ""
    # Check Qt version from the .so
    QT_VER=$(strings /usr/lib/libQt5Core.so.5 2>/dev/null | grep -oP "^5\.\d+\.\d+$" | head -1 || echo "unknown")
    echo "  Qt version: $QT_VER"
'
echo ""

# ---- 4. Check pre-built rm2fb server .so (if present) ----
echo "[4/5] Pre-built rm2fb check"
run_remote '
    SERVER_SO=""
    for p in /opt/lib/librm2fb_server.so* /usr/lib/librm2fb_server.so*; do
        if [ -f "$p" ]; then
            SERVER_SO="$p"
            break
        fi
    done

    if [ -z "$SERVER_SO" ]; then
        echo "  No rm2fb server .so found on tablet"
        echo "  VERDICT: Need to install rm2fb (pre-built or from source)"
    else
        echo "  Found: $SERVER_SO"
        echo "  Size: $(stat -c%s "$SERVER_SO" 2>/dev/null || echo unknown) bytes"
        echo ""
        echo "  Dependency check (ldd):"
        # Check if all dynamic deps are satisfied
        LDD_OUT=$(ldd "$SERVER_SO" 2>&1) || true
        echo "$LDD_OUT" | sed "s/^/    /"
        echo ""
        if echo "$LDD_OUT" | grep -q "not found"; then
            echo "  VERDICT: INCOMPATIBLE — missing shared libraries"
            echo "  The pre-built .so needs libraries not present on this firmware."
            echo "  You need to BUILD RM2FB FROM SOURCE for this firmware."
        else
            echo "  VERDICT: Libraries OK — dependencies satisfied"
            echo "  The pre-built .so can load. You just need the right addresses."
        fi
    fi
'
echo ""

# ---- 5. xochitl binary analysis ----
echo "[5/5] xochitl marker strings"
run_remote '
    XOCHITL=/usr/bin/xochitl
    if [ ! -f "$XOCHITL" ]; then
        echo "  ERROR: /usr/bin/xochitl not found!"
        exit 1
    fi
    echo "  xochitl size: $(stat -c%s "$XOCHITL") bytes"
    echo "  xochitl type: $(file "$XOCHITL" 2>/dev/null | cut -d: -f2 | xargs || echo unknown)"
    echo ""

    # Search for the marker strings rm2fb uses
    echo "  Searching for rm2fb marker strings..."
    echo ""

    # update function marker
    UPDATE_STR=$(strings "$XOCHITL" | grep -i "Unable to complete update.*invalid waveform" | head -1 || echo "")
    if [ -n "$UPDATE_STR" ]; then
        echo "  [FOUND] update marker: \"$UPDATE_STR\""
    else
        echo "  [MISSING] update marker: \"Unable to complete update: invalid waveform\""
        # Try alternates
        ALT=$(strings "$XOCHITL" | grep -i "invalid waveform" | head -1 || echo "")
        [ -n "$ALT" ] && echo "    alternate: \"$ALT\""
        ALT=$(strings "$XOCHITL" | grep -i "waveform" | head -3 || echo "")
        [ -n "$ALT" ] && echo "    waveform strings:" && echo "$ALT" | sed "s/^/      /"
    fi
    echo ""

    # create function marker
    CREATE_STR=$(strings "$XOCHITL" | grep -i "Unable to start generator thread" | head -1 || echo "")
    if [ -n "$CREATE_STR" ]; then
        echo "  [FOUND] create marker: \"$CREATE_STR\""
    else
        echo "  [MISSING] create marker: \"Unable to start generator thread\""
        ALT=$(strings "$XOCHITL" | grep -i "generator thread" | head -1 || echo "")
        [ -n "$ALT" ] && echo "    alternate: \"$ALT\""
        ALT=$(strings "$XOCHITL" | grep -i "generator\|SWTCON\|swtcon" | head -5 || echo "")
        [ -n "$ALT" ] && echo "    related strings:" && echo "$ALT" | sed "s/^/      /"
    fi
    echo ""

    # EPFrameBuffer strings
    echo "  EPFrameBuffer / SWTCON strings:"
    strings "$XOCHITL" | grep -i "EPFrame\|epframe\|SwtFb\|swtfb\|SWTCON\|swtcon\|sendUpdate" | head -10 | sed "s/^/    /" || echo "    (none found)"
    echo ""

    # Check if xochitl is stripped
    if nm "$XOCHITL" 2>/dev/null | head -1 >/dev/null 2>&1; then
        echo "  Symbols: present (nm works)"
    else
        echo "  Symbols: stripped (remarkable-ssh --setup handles this automatically)"
    fi
'

echo ""
echo "=== Summary ==="
echo ""
echo "Next steps depend on the results above:"
echo ""
echo "  If [4/5] says 'Libraries OK':"
echo "    -> Run install.sh (auto-extracts addresses) or: ssh root@$TABLET_IP remarkable-ssh --setup"
echo ""
echo "  If [4/5] says 'INCOMPATIBLE' or no .so found:"
echo "    -> Need to build rm2fb from source. Run: ./scripts/build-rm2fb.sh $TABLET_IP"
echo ""
echo "  If [5/5] marker strings are [MISSING]:"
echo "    -> Firmware may have changed xochitl internals significantly."
echo "    -> rm2fb may need code changes (not just new addresses)."
echo "    -> Check https://github.com/ddvk/remarkable2-framebuffer/issues"
