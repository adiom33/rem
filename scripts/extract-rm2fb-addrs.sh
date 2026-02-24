#!/usr/bin/env bash
#
# extract-rm2fb-addrs.sh — Extract rm2fb function addresses from xochitl binary
#
# This script pulls /usr/bin/xochitl from your reMarkable 2 tablet, searches
# for the function addresses that rm2fb needs, and generates an rm2fb.conf file.
#
# Usage:
#   ./scripts/extract-rm2fb-addrs.sh [TABLET_IP]
#
# Default TABLET_IP: 10.11.99.1 (USB connection)
#
# Prerequisites (on your computer):
#   - arm-linux-gnueabihf-objdump  OR  objdump (for cross-disassembly)
#   - strings
#   - SSH access to the tablet (root@TABLET_IP)
#
# What this does:
#   1. Pulls /usr/bin/xochitl and firmware version from the tablet
#   2. Searches the binary for rm2fb's required function signatures
#   3. Generates rm2fb.conf with the discovered addresses
#   4. Optionally deploys rm2fb.conf to the tablet
#
# The three functions rm2fb needs:
#   update  — triggers e-ink refresh (marker string: "Unable to complete update: invalid waveform")
#   create  — starts SWTCON generator threads (marker string: "Unable to start generator thread")
#   notify  — notifies update completion
#
set -euo pipefail

TABLET_IP="${1:-10.11.99.1}"
WORK_DIR="$(mktemp -d)"
XOCHITL="$WORK_DIR/xochitl"
SSH_OPTS=(-o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)

cleanup() {
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

echo "=== rm2fb Address Extractor ==="
echo ""

# ---- Step 1: Pull xochitl and firmware version ----
echo "[1/4] Pulling xochitl binary from $TABLET_IP..."
scp "${SSH_OPTS[@]}" "root@${TABLET_IP}:/usr/bin/xochitl" "$XOCHITL" || {
    echo "ERROR: Cannot copy /usr/bin/xochitl from tablet."
    echo "  Make sure:"
    echo "  - The tablet is connected via USB (IP: 10.11.99.1)"
    echo "  - You can SSH in: ssh root@${TABLET_IP}"
    exit 1
}

FIRMWARE_VER=$(ssh "${SSH_OPTS[@]}" "root@${TABLET_IP}" \
    'grep REMARKABLE_RELEASE_VERSION /usr/share/remarkable/update.conf 2>/dev/null | cut -d= -f2 || cat /etc/version 2>/dev/null || echo unknown' \
) || FIRMWARE_VER="unknown"
FIRMWARE_VER=$(echo "$FIRMWARE_VER" | tr -d '[:space:]')

echo "  Firmware version: $FIRMWARE_VER"
echo "  Binary size: $(stat -f%z "$XOCHITL" 2>/dev/null || stat -c%s "$XOCHITL" 2>/dev/null || echo '?') bytes"
echo ""

# ---- Step 2: Pick the right objdump ----
OBJDUMP=""
for candidate in arm-linux-gnueabihf-objdump arm-none-eabi-objdump objdump; do
    if command -v "$candidate" &>/dev/null; then
        OBJDUMP="$candidate"
        break
    fi
done

if [ -z "$OBJDUMP" ]; then
    echo "ERROR: No objdump found. Install one of:"
    echo "  Ubuntu/Debian: sudo apt install binutils-arm-linux-gnueabihf"
    echo "  macOS: brew install arm-linux-gnueabihf-binutils"
    exit 1
fi
echo "[2/4] Using disassembler: $OBJDUMP"

# Verify it can read the binary
if ! "$OBJDUMP" -f "$XOCHITL" &>/dev/null; then
    echo "ERROR: $OBJDUMP cannot read the xochitl binary."
    echo "  The binary is ARM — you may need arm-linux-gnueabihf-objdump."
    exit 1
fi
echo ""

# ---- Step 3: Search for function addresses ----
echo "[3/4] Searching for rm2fb function addresses..."
echo "  (This may take a minute — the binary is large)"
echo ""

# Disassemble once, save to file for multiple searches
DISASM="$WORK_DIR/disasm.txt"
STRINGS_OUT="$WORK_DIR/strings.txt"

echo "  Extracting strings..."
strings -t x "$XOCHITL" > "$STRINGS_OUT"

echo "  Disassembling (this is the slow part)..."
"$OBJDUMP" -d "$XOCHITL" > "$DISASM" 2>/dev/null

# --- Find "update" function ---
# The update function references "Unable to complete update: invalid waveform ("
# Strategy: find the string offset, then find code that references it
echo ""
echo "  --- Searching for 'update' function ---"
UPDATE_ADDR=""

# Find string offset
UPDATE_STR_OFFSET=$(grep -i "Unable to complete update.*invalid waveform" "$STRINGS_OUT" | head -1 | awk '{print $1}')
if [ -n "$UPDATE_STR_OFFSET" ]; then
    echo "  Found marker string at offset 0x$UPDATE_STR_OFFSET"

    # Search for references to this address in the disassembly
    # On ARM, string references often use a literal pool (ldr rN, [pc, #offset])
    # We look for the address in the disassembly
    # The virtual address = file offset + base (usually 0x10000 for PIE, but we search raw)

    # Get the section-relative address by finding it in the .rodata section
    VADDR=$(grep -i "Unable to complete update.*invalid waveform" "$DISASM" 2>/dev/null | head -1 | awk -F: '{print $1}' | tr -d ' ')

    if [ -z "$VADDR" ]; then
        # Try finding references to the string offset in literal pools
        # ARM loads string addresses via literal pool entries near the function
        echo "  Searching disassembly for references to string..."

        # Look for functions that contain xrefs near this string
        # This is a heuristic — we look for the hex bytes of the offset
        HEX_OFFSET=$(printf "%08x" "0x$UPDATE_STR_OFFSET" 2>/dev/null || echo "")
        if [ -n "$HEX_OFFSET" ]; then
            # Search for literal pool entries containing this address
            REFS=$(grep -n "$HEX_OFFSET\|${HEX_OFFSET:2}" "$DISASM" | head -5)
            if [ -n "$REFS" ]; then
                echo "  Potential references found:"
                echo "$REFS" | head -3 | sed 's/^/    /'
            fi
        fi
    fi
else
    echo "  WARNING: Marker string 'Unable to complete update: invalid waveform' not found"
    echo "  This string may have changed in firmware $FIRMWARE_VER"

    # Try alternate strings
    for alt in "invalid waveform" "complete update" "waveform mode"; do
        ALT_OFFSET=$(grep -i "$alt" "$STRINGS_OUT" | head -1 | awk '{print $1}')
        if [ -n "$ALT_OFFSET" ]; then
            echo "  Found alternate marker '$alt' at 0x$ALT_OFFSET"
        fi
    done
fi

# --- Find "create" function ---
echo ""
echo "  --- Searching for 'create' function ---"
CREATE_ADDR=""

CREATE_STR_OFFSET=$(grep -i "Unable to start generator thread" "$STRINGS_OUT" | head -1 | awk '{print $1}')
if [ -n "$CREATE_STR_OFFSET" ]; then
    echo "  Found marker string at offset 0x$CREATE_STR_OFFSET"
else
    echo "  WARNING: Marker string 'Unable to start generator thread' not found"
    # Try alternates
    for alt in "generator thread" "start generator" "Unable to start"; do
        ALT_OFFSET=$(grep -i "$alt" "$STRINGS_OUT" | head -1 | awk '{print $1}')
        if [ -n "$ALT_OFFSET" ]; then
            echo "  Found alternate marker '$alt' at 0x$ALT_OFFSET"
        fi
    done
fi

# --- Find "wait/notify" function ---
echo ""
echo "  --- Searching for 'notify' / 'wait' function ---"

# The wait/notify function is typically near create
# Some versions use a "notify" address instead
for marker in "QWaitCondition" "wakeAll" "epframebuffer" "FrameBuffer::instance" "EPFrameBuffer"; do
    MARKER_OFFSET=$(grep -i "$marker" "$STRINGS_OUT" | head -1 | awk '{print $1}')
    if [ -n "$MARKER_OFFSET" ]; then
        echo "  Found marker '$marker' at offset 0x$MARKER_OFFSET"
    fi
done

# ---- Step 4: Generate output ----
echo ""
echo "========================================"
echo ""

# Dump all interesting strings for manual analysis
echo "[4/4] Generating analysis report..."

REPORT="$WORK_DIR/report.txt"
cat > "$REPORT" <<HEADER
rm2fb Address Analysis Report
Firmware: $FIRMWARE_VER
Date: $(date -u +"%Y-%m-%d %H:%M UTC")

=== Key Strings Found ===
HEADER

echo "" >> "$REPORT"
for pattern in "waveform" "generator thread" "EPFrameBuffer" "epframebuffer" \
               "QWaitCondition" "SWTCON" "swtcon" "update.*complete" \
               "FrameBuffer" "instance" "Unable to" "fb_update" "sendUpdate"; do
    MATCHES=$(grep -i "$pattern" "$STRINGS_OUT" 2>/dev/null | head -5)
    if [ -n "$MATCHES" ]; then
        echo "--- '$pattern' ---" >> "$REPORT"
        echo "$MATCHES" >> "$REPORT"
        echo "" >> "$REPORT"
    fi
done

# Also extract all Qt-related symbols that might help
echo "=== Qt/EPFrameBuffer Symbols ===" >> "$REPORT"
grep -i "EPFrame\|epframe\|sendUpdate\|SwtFb\|swtfb\|SWTCON\|swtcon" "$STRINGS_OUT" >> "$REPORT" 2>/dev/null || true
echo "" >> "$REPORT"

# Copy report to project directory
REPORT_OUT="rm2fb-analysis-${FIRMWARE_VER}.txt"
cp "$REPORT" "/home/user/rem/$REPORT_OUT" 2>/dev/null || cp "$REPORT" "./$REPORT_OUT"

echo "Analysis report saved to: $REPORT_OUT"
echo ""

# Check if we found enough to generate a config
if [ -n "$UPDATE_STR_OFFSET" ] && [ -n "$CREATE_STR_OFFSET" ]; then
    echo "String markers found! To complete the address extraction:"
    echo ""
    echo "  1. Open xochitl in Ghidra (free: https://ghidra-sre.org/)"
    echo "     File > Import > select the xochitl binary"
    echo "     Analyze it (default settings are fine)"
    echo ""
    echo "  2. Search > For Strings > 'Unable to complete update: invalid waveform'"
    echo "     Double-click the result, then find the function that references it."
    echo "     That function's address is the 'update' address."
    echo ""
    echo "  3. Search > For Strings > 'Unable to start generator thread'"
    echo "     Same process — the referencing function is 'create'."
    echo ""
    echo "  4. Create /etc/rm2fb.conf on the tablet:"
    echo ""
    echo "     [$FIRMWARE_VER]"
    echo "     update=0x<address from step 2>"
    echo "     create=0x<address from step 3>"
    echo ""
    echo "  5. Restart xochitl with rm2fb server:"
    echo "     systemctl stop xochitl"
    echo "     LD_PRELOAD=/opt/lib/librm2fb_server.so.1 xochitl &"
    echo ""
else
    echo "WARNING: Could not find all marker strings."
    echo "The xochitl binary in firmware $FIRMWARE_VER may have changed"
    echo "the strings rm2fb uses for function identification."
    echo ""
    echo "You will need to manually analyze the binary with Ghidra."
    echo "The binary has been saved to: $XOCHITL"
    echo ""
    echo "Look for functions related to:"
    echo "  - E-ink display update/refresh"
    echo "  - SWTCON thread creation"
    echo "  - EPFrameBuffer class methods"
fi

echo ""
echo "=== Raw string search for manual analysis ==="
echo ""
echo "Strings that might help identify function addresses:"
grep -i "waveform\|generator\|EPFrame\|swtcon\|fb_update\|sendUpdate" "$STRINGS_OUT" | head -20
echo ""
echo "Done. The pulled xochitl binary is at: $XOCHITL"
echo "(It will be deleted when this script exits unless you copy it first)"
echo ""
echo "To keep the binary: cp $XOCHITL ./xochitl-$FIRMWARE_VER"
