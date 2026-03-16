/// Automated rm2fb setup: parse xochitl ELF binary, find function addresses,
/// and generate /etc/rm2fb.conf.
///
/// This eliminates the need for Ghidra or manual reverse engineering.
/// The approach:
/// 1. Parse the ELF32 headers to find .text and .rodata sections
/// 2. Search .rodata for rm2fb's marker strings
/// 3. Compute the virtual addresses of those strings
/// 4. Scan the binary for literal pool entries that reference those VAs
/// 5. Walk backwards from each xref to find the function entry point
/// 6. Write the addresses to /etc/rm2fb.conf

use std::fs;

// ---- Minimal ELF32 parsing ----

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const EI_CLASS_32: u8 = 1;
const EI_DATA_LSB: u8 = 1;
const EM_ARM: u16 = 40;
const SHT_PROGBITS: u32 = 1;
const SHT_STRTAB: u32 = 3;

fn u16_le(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

fn u32_le(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

#[derive(Debug, Clone)]
struct ElfSection {
    name: String,
    sh_type: u32,
    addr: u32,   // virtual address
    offset: u32, // file offset
    size: u32,
}

struct ElfInfo {
    sections: Vec<ElfSection>,
}

fn parse_elf(data: &[u8]) -> Result<ElfInfo, String> {
    if data.len() < 52 {
        return Err("File too small for ELF header".into());
    }
    if data[0..4] != ELF_MAGIC {
        return Err("Not an ELF file".into());
    }
    if data[4] != EI_CLASS_32 {
        return Err("Not a 32-bit ELF (expected ELF32 for ARM)".into());
    }
    if data[5] != EI_DATA_LSB {
        return Err("Not little-endian ELF".into());
    }
    let e_machine = u16_le(data, 18);
    if e_machine != EM_ARM {
        return Err(format!("Not an ARM ELF (e_machine={})", e_machine));
    }

    let e_shoff = u32_le(data, 32) as usize;
    let e_shentsize = u16_le(data, 46) as usize;
    let e_shnum = u16_le(data, 48) as usize;
    let e_shstrndx = u16_le(data, 50) as usize;

    if e_shoff == 0 || e_shnum == 0 {
        return Err("No section headers".into());
    }

    // Read section headers
    let mut raw_sections: Vec<(u32, u32, u32, u32, u32)> = Vec::new(); // (name_idx, type, addr, offset, size)
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        if off + e_shentsize > data.len() {
            break;
        }
        let sh_name = u32_le(data, off);
        let sh_type = u32_le(data, off + 4);
        let sh_addr = u32_le(data, off + 12);
        let sh_offset = u32_le(data, off + 16);
        let sh_size = u32_le(data, off + 20);
        raw_sections.push((sh_name, sh_type, sh_addr, sh_offset, sh_size));
    }

    // Get the section name string table
    let shstrtab = if e_shstrndx < raw_sections.len() {
        let (_, _, _, strtab_off, strtab_size) = raw_sections[e_shstrndx];
        let start = strtab_off as usize;
        let end = start + strtab_size as usize;
        if end <= data.len() {
            &data[start..end]
        } else {
            &[]
        }
    } else {
        &[]
    };

    // Build named sections
    let sections: Vec<ElfSection> = raw_sections
        .iter()
        .map(|&(name_idx, sh_type, addr, offset, size)| {
            let name = if (name_idx as usize) < shstrtab.len() {
                let start = name_idx as usize;
                let end = shstrtab[start..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|p| start + p)
                    .unwrap_or(shstrtab.len());
                String::from_utf8_lossy(&shstrtab[start..end]).into_owned()
            } else {
                String::new()
            };
            ElfSection { name, sh_type, addr, offset, size }
        })
        .collect();

    Ok(ElfInfo { sections })
}

// ---- String search ----

/// Find the virtual address of a string in the ELF binary.
/// Searches all PROGBITS and STRTAB sections (covers .rodata, .data, etc.)
fn find_string_va(data: &[u8], elf: &ElfInfo, needle: &str) -> Option<u32> {
    let needle_bytes = needle.as_bytes();
    for section in &elf.sections {
        if section.sh_type != SHT_PROGBITS && section.sh_type != SHT_STRTAB {
            continue;
        }
        let start = section.offset as usize;
        let end = start + section.size as usize;
        if end > data.len() {
            continue;
        }
        let section_data = &data[start..end];

        // Search for the needle in this section
        if let Some(pos) = find_bytes(section_data, needle_bytes) {
            let va = section.addr + pos as u32;
            return Some(va);
        }
    }
    None
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

// ---- ARM xref scanning ----

/// Find all locations in the binary where a 4-byte little-endian value matching
/// `target_va` appears. These are literal pool entries that reference the target.
fn find_literal_pool_refs(data: &[u8], elf: &ElfInfo, target_va: u32) -> Vec<u32> {
    let target_bytes = target_va.to_le_bytes();
    let mut refs = Vec::new();

    // Search all executable/PROGBITS sections
    for section in &elf.sections {
        if section.sh_type != SHT_PROGBITS {
            continue;
        }
        let start = section.offset as usize;
        let end = start + section.size as usize;
        if end > data.len() {
            continue;
        }

        // Scan for the 4-byte pattern (aligned to 4 bytes for ARM literal pools)
        let mut pos = 0;
        while pos + 4 <= section.size as usize {
            if data[start + pos..start + pos + 4] == target_bytes {
                let ref_va = section.addr + pos as u32;
                refs.push(ref_va);
            }
            pos += 4; // Literal pools are word-aligned
        }
    }

    refs
}

/// Given a literal pool entry VA, find the function that references it.
/// Strategy: scan backwards from the literal pool entry looking for a
/// Thumb PUSH instruction that saves LR (marks a function prologue).
///
/// Thumb-2 PUSH patterns:
///   16-bit: 0xB5xx (PUSH {Rn..., LR}) where xx has bit 0x01 (R0) through 0xFF
///           Specifically: 0xB500-0xB5FF where the instruction saves LR
///   32-bit: 0xE92D xxxx (PUSH.W {registers}) where xxxx has bit 14 set (LR)
fn find_function_entry(data: &[u8], elf: &ElfInfo, literal_pool_va: u32) -> Option<u32> {
    // Find which section this VA belongs to
    let section = elf.sections.iter().find(|s| {
        s.sh_type == SHT_PROGBITS
            && literal_pool_va >= s.addr
            && literal_pool_va < s.addr + s.size
    })?;

    let section_start = section.offset as usize;
    let section_end = section_start + section.size as usize;
    // Validate section bounds against file size
    if section_end > data.len() {
        return None;
    }
    let pool_offset_in_section = (literal_pool_va - section.addr) as usize;

    // In ARM/Thumb, the literal pool is typically right after the function
    // or between functions. The LDR instruction that references the pool
    // is within ~4KB before the pool entry.
    // Walk backwards from the pool entry to find function prologues.

    // First, find which LDR instruction references this literal pool entry.
    // Thumb LDR Rt, [PC, #imm]: encoded as 0x4800 | (Rt << 8) | (imm >> 2)
    // The PC-relative offset is: (imm << 2), and PC = current_addr + 4 (Thumb pipeline)
    // So: literal_pool_addr = (LDR_addr + 4) & ~3 + imm
    //
    // Search backwards up to 4KB for an LDR that targets this pool entry.

    let search_start = if pool_offset_in_section > 4096 {
        pool_offset_in_section - 4096
    } else {
        0
    };

    let mut best_ldr_offset: Option<usize> = None;

    // Search for Thumb LDR Rt, [PC, #imm8*4] (encoding T1: 0x4800-0x4FFF)
    let mut off = search_start;
    while off + 2 <= pool_offset_in_section {
        if section_start + off + 2 > data.len() { break; }
        let insn = u16_le(data, section_start + off);
        if (insn & 0xF800) == 0x4800 {
            // LDR Rt, [PC, #imm8*4]
            let imm8 = (insn & 0xFF) as u32;
            let pc = section.addr + off as u32 + 4; // Thumb PC = addr + 4
            let target = (pc & !3) + (imm8 << 2);
            if target == literal_pool_va {
                best_ldr_offset = Some(off);
                break; // Take the first (closest to pool = most likely)
            }
        }
        off += 2; // Thumb instructions are 2-byte aligned
    }

    // If we didn't find a Thumb-16 LDR, try Thumb-32 LDR.W
    if best_ldr_offset.is_none() {
        off = search_start;
        while off + 4 <= pool_offset_in_section {
            if section_start + off + 4 > data.len() { break; }
            let hw1 = u16_le(data, section_start + off);
            let hw2 = u16_le(data, section_start + off + 2);
            // LDR.W Rt, [PC, #imm12]: 1111 1000 x101 1111 | Rt imm12
            // Encoding: hw1 = 0xF8DF, hw2 = Rt(15:12) imm12(11:0)
            // Or with U=0: hw1 = 0xF85F
            if hw1 == 0xF8DF || hw1 == 0xF85F {
                let imm12 = (hw2 & 0xFFF) as u32;
                let pc = section.addr + off as u32 + 4;
                let target = if hw1 == 0xF8DF {
                    (pc & !3) + imm12 // Add
                } else {
                    (pc & !3).wrapping_sub(imm12) // Subtract
                };
                if target == literal_pool_va {
                    best_ldr_offset = Some(off);
                    break;
                }
            }
            off += 2;
        }
    }

    let ldr_offset = best_ldr_offset?;

    // Now walk backwards from the LDR to find the function prologue.
    // Look for PUSH {... LR} which is the standard function entry.
    // Thumb-16 PUSH: 0xB500-0xB5FF (bit 8 = LR is always set for function entry)
    // Thumb-32 PUSH.W: 0xE92D xxxx where bit 14 of xxxx = LR

    let mut scan = ldr_offset;
    // Don't scan more than 2KB back — functions aren't usually that big
    let scan_limit = if scan > 2048 { scan - 2048 } else { 0 };

    while scan >= scan_limit + 2 {
        scan -= 2;
        if section_start + scan + 2 > data.len() { continue; }
        let insn = u16_le(data, section_start + scan);

        // Thumb-16 PUSH with LR
        if (insn & 0xFF00) == 0xB500 {
            let func_va = section.addr + scan as u32;
            return Some(func_va | 1); // Set bit 0 for Thumb address
        }

        // Check for Thumb-32 PUSH.W (need to check the previous halfword too)
        if scan >= 2 && section_start + scan > 2 {
            if section_start + scan - 2 + 2 <= data.len() {
                let prev_hw = u16_le(data, section_start + scan - 2);
                if prev_hw == 0xE92D {
                    // insn is the register list; check if LR (bit 14) is set
                    if insn & (1 << 14) != 0 {
                        let func_va = section.addr + (scan - 2) as u32;
                        return Some(func_va | 1); // Thumb
                    }
                }
            }
        }
    }

    // Could not find a clean function prologue (PUSH {... LR}).
    // Return None rather than guessing — a wrong address would cause
    // xochitl to crash when rm2fb hooks the function, potentially bricking the device.
    eprintln!("    WARNING: LDR found at offset 0x{:x} but no function prologue nearby", ldr_offset);
    None
}

// ---- Public interface ----

/// Result of the automated address extraction.
pub struct Rm2fbAddresses {
    pub firmware_version: String,
    pub update_addr: Option<u32>,
    pub create_addr: Option<u32>,
    pub update_str: Option<String>,
    pub create_str: Option<String>,
}

/// Parse xochitl and extract rm2fb function addresses.
pub fn extract_addresses(xochitl_path: &str) -> Result<Rm2fbAddresses, String> {
    // Read firmware version
    let firmware_version = if let Ok(contents) = fs::read_to_string("/usr/share/remarkable/update.conf") {
        contents.lines()
            .find_map(|line| line.strip_prefix("REMARKABLE_RELEASE_VERSION="))
            .unwrap_or("unknown")
            .trim()
            .to_string()
    } else if let Ok(ver) = fs::read_to_string("/etc/version") {
        ver.trim().to_string()
    } else {
        "unknown".to_string()
    };

    eprintln!("rm2fb setup: firmware {}", firmware_version);
    eprintln!("rm2fb setup: reading {}...", xochitl_path);

    let data = fs::read(xochitl_path)
        .map_err(|e| format!("Cannot read {}: {}", xochitl_path, e))?;

    eprintln!("rm2fb setup: {} bytes, parsing ELF...", data.len());

    let elf = parse_elf(&data)?;

    eprintln!("rm2fb setup: {} sections found", elf.sections.len());
    for s in &elf.sections {
        if !s.name.is_empty() && (s.name.contains("text") || s.name.contains("rodata")) {
            eprintln!("  {} @ 0x{:08x} ({} bytes)", s.name, s.addr, s.size);
        }
    }

    // ---- Find "update" function ----
    let update_marker = "Unable to complete update: invalid waveform";
    let create_marker = "Unable to start generator thread";

    eprintln!("rm2fb setup: searching for update marker...");
    let update_addr = find_function_for_string(&data, &elf, update_marker);
    let update_str = find_string_va(&data, &elf, update_marker).map(|va| format!("0x{:08x}", va));

    match update_addr {
        Some(addr) => eprintln!("  update function: 0x{:08x}", addr),
        None => {
            eprintln!("  update marker string not found or no xref");
            // Try alternate markers
            for alt in &["invalid waveform", "waveform mode"] {
                if let Some(va) = find_string_va(&data, &elf, alt) {
                    eprintln!("  (alternate '{}' found at VA 0x{:08x})", alt, va);
                }
            }
        }
    }

    eprintln!("rm2fb setup: searching for create marker...");
    let create_addr = find_function_for_string(&data, &elf, create_marker);
    let create_str = find_string_va(&data, &elf, create_marker).map(|va| format!("0x{:08x}", va));

    match create_addr {
        Some(addr) => eprintln!("  create function: 0x{:08x}", addr),
        None => {
            eprintln!("  create marker string not found or no xref");
            for alt in &["generator thread", "Unable to start"] {
                if let Some(va) = find_string_va(&data, &elf, alt) {
                    eprintln!("  (alternate '{}' found at VA 0x{:08x})", alt, va);
                }
            }
        }
    }

    Ok(Rm2fbAddresses {
        firmware_version,
        update_addr,
        create_addr,
        update_str,
        create_str,
    })
}

/// Find the function that references a given marker string.
fn find_function_for_string(data: &[u8], elf: &ElfInfo, marker: &str) -> Option<u32> {
    let string_va = find_string_va(data, elf, marker)?;
    eprintln!("  string '{}...' at VA 0x{:08x}", &marker[..marker.len().min(40)], string_va);

    let refs = find_literal_pool_refs(data, elf, string_va);
    eprintln!("  found {} literal pool reference(s)", refs.len());

    for &ref_va in &refs {
        eprintln!("    literal pool @ 0x{:08x}", ref_va);
        if let Some(func_va) = find_function_entry(data, elf, ref_va) {
            eprintln!("    -> function entry @ 0x{:08x}", func_va);
            return Some(func_va);
        }
    }

    None
}

/// Write /etc/rm2fb.conf with the extracted addresses.
pub fn write_rm2fb_conf(addrs: &Rm2fbAddresses) -> Result<(), String> {
    let update = addrs.update_addr
        .ok_or("Could not find 'update' function address")?;
    let create = addrs.create_addr
        .ok_or("Could not find 'create' function address")?;

    let conf = format!(
        "# rm2fb configuration for firmware {}\n\
         # Auto-generated by remarkable-ssh --setup\n\
         # update marker: {}\n\
         # create marker: {}\n\
         \n\
         [{}]\n\
         update=0x{:08x}\n\
         create=0x{:08x}\n",
        addrs.firmware_version,
        addrs.update_str.as_deref().unwrap_or("?"),
        addrs.create_str.as_deref().unwrap_or("?"),
        addrs.firmware_version,
        update & !1, // Clear Thumb bit for the config file
        create & !1,
    );

    eprintln!("Writing /etc/rm2fb.conf:");
    eprintln!("{}", conf);

    fs::write("/etc/rm2fb.conf", &conf)
        .map_err(|e| format!("Cannot write /etc/rm2fb.conf: {}", e))?;

    eprintln!("rm2fb.conf written successfully.");
    Ok(())
}

/// Full setup: extract addresses and write config.
pub fn run_setup() -> Result<(), String> {
    eprintln!("=== remarkable-ssh: rm2fb auto-setup ===");
    eprintln!();

    let addrs = extract_addresses("/usr/bin/xochitl")?;

    if addrs.update_addr.is_none() || addrs.create_addr.is_none() {
        eprintln!();
        eprintln!("Could not find all required function addresses.");
        eprintln!("This firmware may have changed the marker strings.");
        eprintln!();
        if addrs.update_addr.is_some() {
            eprintln!("  update: 0x{:08x} (found)", addrs.update_addr.unwrap());
        } else {
            eprintln!("  update: NOT FOUND");
        }
        if addrs.create_addr.is_some() {
            eprintln!("  create: 0x{:08x} (found)", addrs.create_addr.unwrap());
        } else {
            eprintln!("  create: NOT FOUND");
        }
        eprintln!();
        eprintln!("You may need to find the addresses manually with Ghidra.");
        eprintln!("See: ./scripts/extract-rm2fb-addrs.sh");
        return Err("Incomplete address extraction".into());
    }

    write_rm2fb_conf(&addrs)?;

    // Check if rm2fb server .so exists
    let server_paths = [
        "/opt/lib/librm2fb_server.so.1",
        "/opt/lib/librm2fb_server.so",
        "/usr/lib/librm2fb_server.so.1",
    ];
    let has_server = server_paths.iter().any(|p| std::path::Path::new(p).exists());

    if !has_server {
        eprintln!();
        eprintln!("WARNING: rm2fb server .so not found on this device.");
        eprintln!("You still need to install it:");
        eprintln!("  1. Download from https://github.com/ddvk/remarkable2-framebuffer/releases");
        eprintln!("  2. Copy to /opt/lib/librm2fb_server.so.1");
        eprintln!();
    }

    eprintln!();
    eprintln!("=== Setup complete ===");
    eprintln!();
    eprintln!("To activate rm2fb:");
    eprintln!("  systemctl stop xochitl");
    eprintln!("  LD_PRELOAD=/opt/lib/librm2fb_server.so.1 xochitl &");
    eprintln!();
    eprintln!("Then run remarkable-ssh — it will auto-detect rm2fb.");

    Ok(())
}
