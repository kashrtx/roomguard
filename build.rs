//! Build script for RoomGuard.
//!
//! On Windows targets, embeds the multi-resolution application icon
//! (`assets/icon/icon.ico`) and `VS_VERSIONINFO` metadata directly into a
//! standard COFF `.rsrc` object file with zero external tool dependencies
//! (no `rc.exe` or `windres.exe` required, so it works identically on MSVC,
//! GNU MinGW, paths containing spaces, and CI runners).

fn main() {
    // Release builds made by GitHub Actions set this so the update checker
    // knows which repository its releases come from.
    println!("cargo:rerun-if-env-changed=ROOMGUARD_REPO");
    println!("cargo:rerun-if-changed=assets/icon/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        win_rsrc::embed_windows_resources();
    }
}

mod win_rsrc {
    use std::path::PathBuf;

    const RT_ICON: u32 = 3;
    const RT_GROUP_ICON: u32 = 14;
    const RT_VERSION: u32 = 16;
    const LANG_EN_US: u32 = 0x0409;

    pub fn embed_windows_resources() {
        let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let ico_path = manifest_dir.join("assets").join("icon").join("icon.ico");
        let ico = std::fs::read(&ico_path)
            .unwrap_or_else(|e| panic!("failed to read Windows icon {}: {}", ico_path.display(), e));

        let (icon_entries, group_icon) = parse_ico(&ico);
        let version_info = build_version_info();

        let types: Vec<(u32, Vec<(u32, Vec<u8>)>)> = vec![
            (RT_ICON, icon_entries),
            (RT_GROUP_ICON, vec![(1, group_icon)]),
            (RT_VERSION, vec![(1, version_info)]),
        ];

        let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".to_string());
        let obj = build_coff_rsrc(&types, &arch);

        let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        let obj_path = out_dir.join("roomguard_res.o");
        std::fs::write(&obj_path, &obj)
            .unwrap_or_else(|e| panic!("failed to write {}: {}", obj_path.display(), e));

        println!("cargo:rustc-link-arg-bin=roomguard={}", obj_path.display());
    }

    fn u16_le(b: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([b[off], b[off + 1]])
    }

    fn u32_le(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }

    fn parse_ico(ico: &[u8]) -> (Vec<(u32, Vec<u8>)>, Vec<u8>) {
        assert!(ico.len() >= 6, "icon.ico is too small");
        let reserved = u16_le(ico, 0);
        let kind = u16_le(ico, 2);
        let count = u16_le(ico, 4) as usize;
        assert!(reserved == 0 && kind == 1 && count > 0, "icon.ico has invalid ICO header");
        assert!(ico.len() >= 6 + count * 16, "icon.ico directory truncated");

        let mut icons = Vec::with_capacity(count);
        let mut grp = Vec::with_capacity(6 + count * 14);
        grp.extend_from_slice(&0u16.to_le_bytes());
        grp.extend_from_slice(&1u16.to_le_bytes());
        grp.extend_from_slice(&(count as u16).to_le_bytes());

        for i in 0..count {
            let e = 6 + i * 16;
            let width = ico[e];
            let height = ico[e + 1];
            let color_count = ico[e + 2];
            let res = ico[e + 3];
            let planes = u16_le(ico, e + 4).max(1);
            let bpp = u16_le(ico, e + 6);
            let bpp = if bpp == 0 { 32 } else { bpp };
            let bytes_in_res = u32_le(ico, e + 8) as usize;
            let image_off = u32_le(ico, e + 12) as usize;
            assert!(
                image_off + bytes_in_res <= ico.len(),
                "icon.ico entry {} out of bounds",
                i
            );
            let id = (i + 1) as u32;
            icons.push((id, ico[image_off..image_off + bytes_in_res].to_vec()));

            grp.push(width);
            grp.push(height);
            grp.push(color_count);
            grp.push(res);
            grp.extend_from_slice(&planes.to_le_bytes());
            grp.extend_from_slice(&bpp.to_le_bytes());
            grp.extend_from_slice(&(bytes_in_res as u32).to_le_bytes());
            grp.extend_from_slice(&(id as u16).to_le_bytes());
        }

        (icons, grp)
    }

    fn utf16z(s: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity((s.len() + 1) * 2);
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn pad4(buf: &mut Vec<u8>) {
        while buf.len() % 4 != 0 {
            buf.push(0);
        }
    }

    fn ver_block(key: &str, value: &[u8], value_len_field: u16, is_text: bool, children: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u16.to_le_bytes()); // wLength placeholder
        out.extend_from_slice(&value_len_field.to_le_bytes());
        out.extend_from_slice(&(if is_text { 1u16 } else { 0u16 }).to_le_bytes());
        out.extend_from_slice(&utf16z(key));
        pad4(&mut out);
        if !value.is_empty() {
            out.extend_from_slice(value);
        }
        for child in children {
            pad4(&mut out);
            out.extend_from_slice(child);
        }
        let len = out.len() as u16;
        out[0..2].copy_from_slice(&len.to_le_bytes());
        out
    }

    fn string_entry(key: &str, val: &str) -> Vec<u8> {
        let bytes = utf16z(val);
        let words = (bytes.len() / 2) as u16;
        ver_block(key, &bytes, words, true, &[])
    }

    fn build_version_info() -> Vec<u8> {
        let ver_str = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
        let major: u16 = std::env::var("CARGO_PKG_VERSION_MAJOR").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minor: u16 = std::env::var("CARGO_PKG_VERSION_MINOR").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        let patch: u16 = std::env::var("CARGO_PKG_VERSION_PATCH").ok().and_then(|s| s.parse().ok()).unwrap_or(0);

        let ver_ms: u32 = ((major as u32) << 16) | (minor as u32);
        let ver_ls: u32 = (patch as u32) << 16;

        // VS_FIXEDFILEINFO (52 bytes)
        let mut fixed = Vec::with_capacity(52);
        for w in [
            0xFEEF_04BDu32, // dwSignature
            0x0001_0000,    // dwStrucVersion
            ver_ms,         // dwFileVersionMS
            ver_ls,         // dwFileVersionLS
            ver_ms,         // dwProductVersionMS
            ver_ls,         // dwProductVersionLS
            0x0000_003F,    // dwFileFlagsMask
            0x0000_0000,    // dwFileFlags
            0x0004_0004,    // dwFileOS (VOS_NT_WINDOWS32)
            0x0000_0001,    // dwFileType (VFT_APP)
            0x0000_0000,    // dwFileSubtype
            0x0000_0000,    // dwFileDateMS
            0x0000_0000,    // dwFileDateLS
        ] {
            fixed.extend_from_slice(&w.to_le_bytes());
        }

        let strings = vec![
            string_entry("CompanyName", "Kaushal Bhingaradia"),
            string_entry("FileDescription", "RoomGuard"),
            string_entry("FileVersion", &ver_str),
            string_entry("InternalName", "roomguard"),
            string_entry("LegalCopyright", "Copyright 2026 Kaushal Bhingaradia. Apache License 2.0."),
            string_entry("OriginalFilename", "roomguard.exe"),
            string_entry("ProductName", "RoomGuard"),
            string_entry("ProductVersion", &ver_str),
        ];
        let str_table = ver_block("040904b0", &[], 0, true, &strings);
        let str_file_info = ver_block("StringFileInfo", &[], 0, true, &[str_table]);

        let trans_val: [u8; 4] = [0x09, 0x04, 0xB0, 0x04]; // 0x0409, 0x04B0
        let var_trans = ver_block("Translation", &trans_val, 4, false, &[]);
        let var_file_info = ver_block("VarFileInfo", &[], 0, true, &[var_trans]);

        ver_block("VS_VERSION_INFO", &fixed, fixed.len() as u16, false, &[str_file_info, var_file_info])
    }

    fn build_coff_rsrc(types: &[(u32, Vec<(u32, Vec<u8>)>)], arch: &str) -> Vec<u8> {
        let (machine, reloc_type): (u16, u16) = match arch {
            "x86" => (0x014C, 0x0007),     // IMAGE_FILE_MACHINE_I386, IMAGE_REL_I386_DIR32NB
            "aarch64" => (0xAA64, 0x0002), // IMAGE_FILE_MACHINE_ARM64, IMAGE_REL_ARM64_ADDR32NB
            _ => (0x8664, 0x0003),         // IMAGE_FILE_MACHINE_AMD64, IMAGE_REL_AMD64_ADDR32NB
        };

        let mut rsrc = Vec::new();
        let mut relocs: Vec<u32> = Vec::new();

        // Compute deterministic offsets for the 3-level resource directory tree.
        let total_items: usize = types.iter().map(|(_, items)| items.len()).sum();
        let l1_size = 16 + types.len() * 8;
        let mut l2_offsets = Vec::with_capacity(types.len());
        let mut cur = l1_size;
        for (_, items) in types {
            l2_offsets.push(cur);
            cur += 16 + items.len() * 8;
        }
        let l3_start = cur;
        cur += total_items * (16 + 8);
        let data_entries_start = cur;
        cur += total_items * 16;
        let payloads_start = (cur + 3) & !3;

        // Level 1 (Type directory)
        push_dir_hdr(&mut rsrc, types.len() as u16);
        for (ti, (type_id, _)) in types.iter().enumerate() {
            rsrc.extend_from_slice(&type_id.to_le_bytes());
            rsrc.extend_from_slice(&(0x8000_0000u32 | l2_offsets[ti] as u32).to_le_bytes());
        }

        // Level 2 (Name/ID directories)
        let mut item_idx = 0usize;
        for (_, items) in types {
            push_dir_hdr(&mut rsrc, items.len() as u16);
            for (res_id, _) in items {
                let l3_off = l3_start + item_idx * 24;
                rsrc.extend_from_slice(&res_id.to_le_bytes());
                rsrc.extend_from_slice(&(0x8000_0000u32 | l3_off as u32).to_le_bytes());
                item_idx += 1;
            }
        }

        // Level 3 (Language directories)
        for i in 0..total_items {
            push_dir_hdr(&mut rsrc, 1);
            let de_off = data_entries_start + i * 16;
            rsrc.extend_from_slice(&LANG_EN_US.to_le_bytes());
            rsrc.extend_from_slice(&(de_off as u32).to_le_bytes());
        }

        // Compute payload offsets and emit IMAGE_RESOURCE_DATA_ENTRY rows
        let mut payload_off = payloads_start;
        for (_, items) in types {
            for (_, data) in items {
                relocs.push(rsrc.len() as u32);
                rsrc.extend_from_slice(&(payload_off as u32).to_le_bytes());
                rsrc.extend_from_slice(&(data.len() as u32).to_le_bytes());
                rsrc.extend_from_slice(&0u32.to_le_bytes()); // CodePage
                rsrc.extend_from_slice(&0u32.to_le_bytes()); // Reserved
                payload_off = (payload_off + data.len() + 3) & !3;
            }
        }
        pad4(&mut rsrc);
        assert_eq!(rsrc.len(), payloads_start);

        // Emit raw payloads
        for (_, items) in types {
            for (_, data) in items {
                rsrc.extend_from_slice(data);
                pad4(&mut rsrc);
            }
        }

        // Assemble COFF object
        let raw_off = 20u32 + 40u32;
        let reloc_off = raw_off + rsrc.len() as u32;
        let sym_off = reloc_off + (relocs.len() as u32) * 10;

        let mut obj = Vec::with_capacity(sym_off as usize + 18 + 4);
        // IMAGE_FILE_HEADER (20 bytes)
        obj.extend_from_slice(&machine.to_le_bytes());
        obj.extend_from_slice(&1u16.to_le_bytes()); // NumberOfSections
        obj.extend_from_slice(&0u32.to_le_bytes()); // TimeDateStamp
        obj.extend_from_slice(&sym_off.to_le_bytes()); // PointerToSymbolTable
        obj.extend_from_slice(&1u32.to_le_bytes()); // NumberOfSymbols
        obj.extend_from_slice(&0u16.to_le_bytes()); // SizeOfOptionalHeader
        obj.extend_from_slice(&0u16.to_le_bytes()); // Characteristics

        // IMAGE_SECTION_HEADER ".rsrc" (40 bytes)
        obj.extend_from_slice(b".rsrc\0\0\0");
        obj.extend_from_slice(&0u32.to_le_bytes()); // VirtualSize
        obj.extend_from_slice(&0u32.to_le_bytes()); // VirtualAddress
        obj.extend_from_slice(&(rsrc.len() as u32).to_le_bytes()); // SizeOfRawData
        obj.extend_from_slice(&raw_off.to_le_bytes()); // PointerToRawData
        obj.extend_from_slice(&reloc_off.to_le_bytes()); // PointerToRelocations
        obj.extend_from_slice(&0u32.to_le_bytes()); // PointerToLinenumbers
        obj.extend_from_slice(&(relocs.len() as u16).to_le_bytes()); // NumberOfRelocations
        obj.extend_from_slice(&0u16.to_le_bytes()); // NumberOfLinenumbers
        obj.extend_from_slice(&0x4000_0040u32.to_le_bytes()); // CNT_INITIALIZED_DATA | MEM_READ

        // Section data
        obj.extend_from_slice(&rsrc);

        // Relocations (10 bytes each)
        for off in relocs {
            obj.extend_from_slice(&off.to_le_bytes());
            obj.extend_from_slice(&0u32.to_le_bytes()); // SymbolTableIndex 0 (".rsrc")
            obj.extend_from_slice(&reloc_type.to_le_bytes());
        }

        // Symbol table: 1 static symbol for ".rsrc" (18 bytes)
        obj.extend_from_slice(b".rsrc\0\0\0");
        obj.extend_from_slice(&0u32.to_le_bytes()); // Value
        obj.extend_from_slice(&1i16.to_le_bytes()); // SectionNumber (1-based)
        obj.extend_from_slice(&0u16.to_le_bytes()); // Type
        obj.push(3u8); // IMAGE_SYM_CLASS_STATIC
        obj.push(0u8); // NumberOfAuxSymbols

        // Empty COFF string table (4-byte length = 4)
        obj.extend_from_slice(&4u32.to_le_bytes());

        obj
    }

    fn push_dir_hdr(buf: &mut Vec<u8>, id_entries: u16) {
        buf.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
        buf.extend_from_slice(&0u32.to_le_bytes()); // TimeDateStamp
        buf.extend_from_slice(&0u16.to_le_bytes()); // MajorVersion
        buf.extend_from_slice(&0u16.to_le_bytes()); // MinorVersion
        buf.extend_from_slice(&0u16.to_le_bytes()); // NumberOfNamedEntries
        buf.extend_from_slice(&id_entries.to_le_bytes()); // NumberOfIdEntries
    }
}
