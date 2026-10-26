// Structural shared-image fixtures exercise all six assembly targets. They do
// not qualify runtime loading; the selected-artifact test uses the real PyO3 module.
fn u16_at(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn u32_at(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn u64_at(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn extension_image(target: &str) -> Vec<u8> {
    let arm = target.starts_with("aarch64-");
    if target.ends_with("apple-darwin") {
        let mut bytes = vec![0; 168];
        u32_at(&mut bytes, 0, object::macho::MH_MAGIC_64);
        u32_at(&mut bytes, 4, if arm { 0x0100_000c } else { 0x0100_0007 });
        u32_at(&mut bytes, 12, object::macho::MH_DYLIB);
        u32_at(&mut bytes, 16, 2);
        u32_at(&mut bytes, 20, 104);
        u32_at(&mut bytes, 24, object::macho::MH_DYLDLINK);
        u32_at(&mut bytes, 32, object::macho::LC_SYMTAB);
        u32_at(&mut bytes, 36, 24);
        u32_at(&mut bytes, 40, 136);
        u32_at(&mut bytes, 44, 1);
        u32_at(&mut bytes, 48, 152);
        u32_at(&mut bytes, 52, 16);
        u32_at(&mut bytes, 56, object::macho::LC_DYSYMTAB);
        u32_at(&mut bytes, 60, 80);
        u32_at(&mut bytes, 76, 1);
        u32_at(&mut bytes, 136, 1);
        bytes[140] = object::macho::N_ABS | object::macho::N_EXT;
        u64_at(&mut bytes, 144, 0x1000);
        bytes[152..].copy_from_slice(b"\0_PyInit_ait_py\0");
        bytes
    } else if target.ends_with("linux-gnu") {
        let mut bytes = vec![0; 392];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        u16_at(&mut bytes, 16, object::elf::ET_DYN);
        u16_at(
            &mut bytes,
            18,
            if arm {
                object::elf::EM_AARCH64
            } else {
                object::elf::EM_X86_64
            },
        );
        u32_at(&mut bytes, 20, 1);
        u64_at(&mut bytes, 40, 64);
        u16_at(&mut bytes, 52, 64);
        u16_at(&mut bytes, 58, 64);
        u16_at(&mut bytes, 60, 4);
        u16_at(&mut bytes, 62, 1);
        u32_at(&mut bytes, 128 + 4, object::elf::SHT_STRTAB);
        u64_at(&mut bytes, 128 + 24, 320);
        u64_at(&mut bytes, 128 + 32, 15);
        u64_at(&mut bytes, 128 + 48, 1);
        u32_at(&mut bytes, 192 + 4, object::elf::SHT_DYNSYM);
        u64_at(&mut bytes, 192 + 24, 336);
        u64_at(&mut bytes, 192 + 32, 48);
        u32_at(&mut bytes, 192 + 40, 1);
        u32_at(&mut bytes, 192 + 44, 1);
        u64_at(&mut bytes, 192 + 48, 8);
        u64_at(&mut bytes, 192 + 56, 24);
        u32_at(&mut bytes, 256 + 4, object::elf::SHT_PROGBITS);
        u64_at(
            &mut bytes,
            256 + 8,
            (object::elf::SHF_ALLOC | object::elf::SHF_EXECINSTR).into(),
        );
        u64_at(&mut bytes, 256 + 16, 0x1000);
        u64_at(&mut bytes, 256 + 24, 384);
        u64_at(&mut bytes, 256 + 32, 4);
        u64_at(&mut bytes, 256 + 48, 4);
        bytes[320..335].copy_from_slice(b"\0PyInit_ait_py\0");
        u32_at(&mut bytes, 360, 1);
        bytes[364] = (object::elf::STB_GLOBAL << 4) | object::elf::STT_FUNC;
        u16_at(&mut bytes, 366, 3);
        u64_at(&mut bytes, 368, 0x1000);
        u64_at(&mut bytes, 376, 4);
        bytes[384..388].copy_from_slice(if arm {
            b"\xc0\x03\x5f\xd6"
        } else {
            b"\xc3\0\0\0"
        });
        bytes
    } else {
        assert!(target.ends_with("windows-msvc"));
        let mut bytes = vec![0; 1024];
        bytes[..2].copy_from_slice(b"MZ");
        u32_at(&mut bytes, 60, 64);
        bytes[64..68].copy_from_slice(b"PE\0\0");
        u16_at(
            &mut bytes,
            68,
            if arm {
                object::pe::IMAGE_FILE_MACHINE_ARM64
            } else {
                object::pe::IMAGE_FILE_MACHINE_AMD64
            },
        );
        u16_at(&mut bytes, 70, 1);
        u16_at(&mut bytes, 84, 240);
        u16_at(
            &mut bytes,
            86,
            object::pe::IMAGE_FILE_DLL | object::pe::IMAGE_FILE_EXECUTABLE_IMAGE,
        );
        u16_at(&mut bytes, 88, object::pe::IMAGE_NT_OPTIONAL_HDR64_MAGIC);
        u64_at(&mut bytes, 88 + 24, 0x180000000);
        u32_at(&mut bytes, 88 + 32, 0x1000);
        u32_at(&mut bytes, 88 + 36, 0x200);
        u32_at(&mut bytes, 88 + 56, 0x2000);
        u32_at(&mut bytes, 88 + 60, 0x200);
        u32_at(&mut bytes, 88 + 108, 16);
        u32_at(&mut bytes, 88 + 112, 0x1000);
        u32_at(&mut bytes, 88 + 116, 0x100);
        bytes[328..334].copy_from_slice(b".edata");
        u32_at(&mut bytes, 328 + 8, 0x200);
        u32_at(&mut bytes, 328 + 12, 0x1000);
        u32_at(&mut bytes, 328 + 16, 0x200);
        u32_at(&mut bytes, 328 + 20, 0x200);
        u32_at(
            &mut bytes,
            328 + 36,
            object::pe::IMAGE_SCN_CNT_INITIALIZED_DATA | object::pe::IMAGE_SCN_MEM_READ,
        );
        for (offset, value) in [
            (12, 0x1050),
            (16, 1),
            (20, 1),
            (24, 1),
            (28, 0x1040),
            (32, 0x1044),
            (36, 0x1048),
            (64, 0x1100),
            (68, 0x1060),
        ] {
            u32_at(&mut bytes, 512 + offset, value);
        }
        bytes[512 + 80..512 + 91].copy_from_slice(b"ait_py.pyd\0");
        bytes[512 + 96..512 + 110].copy_from_slice(b"PyInit_ait_py\0");
        bytes
    }
}
