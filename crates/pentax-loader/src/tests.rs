// SPDX-License-Identifier: GPL-2.0-or-later

use super::*;

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(elf_type: u16, ph_count: u16) -> Vec<u8> {
    let mut bytes = vec![0; ELF_HEADER_SIZE + usize::from(ph_count) * PROGRAM_HEADER_SIZE];
    bytes[..4].copy_from_slice(&ELF_MAGIC);
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    bytes[7] = 9;
    bytes[8] = 2;
    put_u16(&mut bytes, 16, elf_type);
    put_u16(&mut bytes, 18, 62);
    put_u32(&mut bytes, 20, 1);
    put_u64(&mut bytes, 24, 0x400000);
    if ph_count != 0 {
        put_u64(&mut bytes, 32, ELF_HEADER_SIZE as u64);
        put_u16(&mut bytes, 54, PROGRAM_HEADER_SIZE as u16);
    }
    put_u16(&mut bytes, 52, ELF_HEADER_SIZE as u16);
    put_u16(&mut bytes, 56, ph_count);
    bytes
}

fn segment_header(bytes: &mut [u8], index: usize, values: [u64; 7], kind: u32) {
    let start = ELF_HEADER_SIZE + index * PROGRAM_HEADER_SIZE;
    put_u32(bytes, start, kind);
    put_u32(bytes, start + 4, u32::try_from(values[0]).unwrap());
    for (index, value) in values[1..].iter().enumerate() {
        put_u64(bytes, start + 8 + index * 8, *value);
    }
}

fn executable() -> Vec<u8> {
    let mut bytes = header(2, 1);
    bytes.resize(0x104, 0);
    bytes[0x100..].copy_from_slice(&[0x90, 0x90, 0xeb, 0xfe]);
    segment_header(
        &mut bytes,
        0,
        [5, 0x100, 0x400100, 0, 4, 0x20, 0x100],
        PT_LOAD,
    );
    bytes
}

#[test]
fn parses_borrowed_load_bytes_and_metadata() {
    let bytes = executable();
    let image = parse(&bytes).unwrap();
    assert_eq!(image.entry, 0x400000);
    assert_eq!(image.elf_type, ElfType::Executable);
    assert_eq!(image.os_abi, 9);
    assert_eq!(image.abi_version, 2);
    assert_eq!(image.segments.len(), 1);
    let segment = image.segments[0];
    assert_eq!(segment.vaddr, 0x400100);
    assert_eq!(segment.mem_size, 0x20);
    assert_eq!(segment.file_offset, 0x100);
    assert_eq!(segment.file_bytes, &[0x90, 0x90, 0xeb, 0xfe]);
    assert_eq!(segment.file_bytes.as_ptr(), bytes[0x100..].as_ptr());
    assert_eq!(segment.align, 0x100);
    assert_eq!(segment.zero_fill_size(), Some(0x1c));
    assert!(segment.flags.readable());
    assert!(!segment.flags.writable());
    assert!(segment.flags.executable());
}

#[test]
fn accepts_exactly_supported_executable_types() {
    for elf_type in [2, 3, 0xfe00, 0xfe10, 0xfe18] {
        assert_eq!(
            parse(&header(elf_type, 0)).unwrap().elf_type.raw(),
            elf_type
        );
    }
    for elf_type in [0, 1, 4, 0xfe0c, 0xfeff, 0xffff] {
        assert_eq!(
            parse(&header(elf_type, 0)),
            Err(LoadError::UnsupportedElfType(elf_type))
        );
    }
}

#[test]
fn keeps_dynamic_and_unknown_program_headers_without_mapping_them() {
    let mut bytes = header(0xfe10, 3);
    bytes.resize(0x120, 0);
    let flags = 0x10000006;
    segment_header(&mut bytes, 0, [5, 0x100, 0x400100, 0, 4, 4, 1], PT_LOAD);
    segment_header(
        &mut bytes,
        1,
        [flags, 0x108, 0x400108, 0x1234, 8, 8, 8],
        PT_DYNAMIC,
    );
    segment_header(&mut bytes, 2, [0, 0x110, 0, 0, 16, 0, 0], 0x61000000);
    bytes[0x108..0x110].copy_from_slice(&7u64.to_le_bytes());
    let image = parse(&bytes).unwrap();
    assert_eq!(image.segments.len(), 1);
    assert_eq!(image.program_headers.len(), 3);
    let dynamic = image.program_headers[1];
    assert_eq!(dynamic.kind, PT_DYNAMIC);
    assert_eq!(dynamic.flags.raw(), flags as u32);
    assert!(dynamic.flags.writable());
    assert_eq!(dynamic.physical_address, 0x1234);
    assert_eq!(dynamic.file_bytes, 7u64.to_le_bytes());
    assert_eq!(image.program_headers[2].kind, 0x61000000);
    assert_eq!(image.program_headers[2].mem_size, 0);
}

#[test]
fn recognizes_self_without_claiming_to_extract_it() {
    assert_eq!(parse(&SELF_MAGIC), Err(LoadError::UnsupportedSelf));
    let mut container = vec![0; 0x100];
    container[..4].copy_from_slice(&SELF_MAGIC);
    assert_eq!(parse(&container), Err(LoadError::UnsupportedSelf));
    assert!(
        LoadError::UnsupportedSelf
            .to_string()
            .contains("unsupported")
    );
}

#[test]
fn rejects_every_truncated_prefix_without_panicking() {
    let bytes = executable();
    for prefix_len in 0..bytes.len() {
        assert!(
            parse(&bytes[..prefix_len]).is_err(),
            "accepted prefix {prefix_len}"
        );
    }
}

#[test]
fn validates_signature_and_encoding_before_parsing_fields() {
    let baseline = header(2, 0);
    let changes = [
        (0, 0, LoadError::InvalidMagic),
        (4, 1, LoadError::UnsupportedClass(1)),
        (5, 2, LoadError::UnsupportedEndianness(2)),
        (6, 0, LoadError::UnsupportedIdentificationVersion(0)),
    ];
    for (index, value, expected) in changes {
        let mut bytes = baseline.clone();
        bytes[index] = value;
        assert_eq!(parse(&bytes), Err(expected));
    }
    let mut bytes = baseline.clone();
    put_u16(&mut bytes, 18, 183);
    assert_eq!(parse(&bytes), Err(LoadError::UnsupportedMachine(183)));
    let mut bytes = baseline;
    put_u32(&mut bytes, 20, 2);
    assert_eq!(parse(&bytes), Err(LoadError::UnsupportedVersion(2)));
}

#[test]
fn checks_declared_header_sizes() {
    let mut bytes = executable();
    put_u16(&mut bytes, 52, 63);
    assert_eq!(parse(&bytes), Err(LoadError::InvalidHeaderSize(63)));
    put_u16(&mut bytes, 52, 64);
    for size in [0, 8, 55, 57, u16::MAX] {
        put_u16(&mut bytes, 54, size);
        assert_eq!(
            parse(&bytes),
            Err(LoadError::InvalidProgramHeaderSize(size))
        );
    }
}

#[test]
fn reports_extended_numbering_explicitly() {
    let mut bytes = header(2, 0);
    put_u16(&mut bytes, 56, 0xffff);
    assert_eq!(
        parse(&bytes),
        Err(LoadError::UnsupportedExtendedProgramHeaderCount)
    );
}

#[test]
fn rejects_program_table_overflow_and_out_of_bounds() {
    let mut bytes = executable();
    put_u64(&mut bytes, 32, u64::MAX - 16);
    assert!(matches!(
        parse(&bytes),
        Err(LoadError::RangeOverflow {
            context: "program header table",
            ..
        })
    ));
    put_u64(&mut bytes, 32, 0x1000);
    assert!(matches!(
        parse(&bytes),
        Err(LoadError::Truncated {
            context: "program header table",
            ..
        })
    ));
    put_u64(&mut bytes, 32, 64);
    put_u16(&mut bytes, 56, u16::MAX - 1);
    assert!(matches!(
        parse(&bytes),
        Err(LoadError::Truncated {
            context: "program header table",
            ..
        })
    ));
}

#[test]
fn rejects_program_table_overlapping_the_elf_header() {
    for offset in [0, 1, 8, 63] {
        let mut bytes = executable();
        put_u64(&mut bytes, 32, offset);
        assert_eq!(
            parse(&bytes),
            Err(LoadError::ProgramHeaderTableOverlapsHeader { offset })
        );
    }
}

#[test]
fn rejects_load_file_size_larger_than_memory() {
    let mut bytes = executable();
    put_u64(&mut bytes, 64 + 40, 3);
    assert_eq!(
        parse(&bytes),
        Err(LoadError::SegmentFileSizeExceedsMemory {
            index: 0,
            file_size: 4,
            mem_size: 3
        })
    );
}

#[test]
fn rejects_segment_file_range_overflow_and_truncation() {
    let mut bytes = executable();
    put_u64(&mut bytes, 64 + 48, 1);
    put_u64(&mut bytes, 64 + 8, u64::MAX - 2);
    assert!(matches!(
        parse(&bytes),
        Err(LoadError::RangeOverflow {
            context: "program segment",
            ..
        })
    ));
    put_u64(&mut bytes, 64 + 8, 0x102);
    assert!(matches!(
        parse(&bytes),
        Err(LoadError::Truncated {
            context: "program segment",
            ..
        })
    ));
}

#[test]
fn rejects_memory_address_overflow_before_allocating_any_memory() {
    let mut bytes = executable();
    put_u64(&mut bytes, 64 + 16, u64::MAX - 16);
    assert!(matches!(
        parse(&bytes),
        Err(LoadError::RangeOverflow {
            context: "load segment memory",
            ..
        })
    ));
}

#[test]
fn validates_load_alignment_and_congruence() {
    let mut bytes = executable();
    for align in [3, 7, 24, u64::MAX] {
        put_u64(&mut bytes, 64 + 48, align);
        assert_eq!(
            parse(&bytes),
            Err(LoadError::InvalidSegmentAlignment { index: 0, align })
        );
    }
    put_u64(&mut bytes, 64 + 48, 0x100);
    put_u64(&mut bytes, 64 + 16, 0x400101);
    assert!(matches!(
        parse(&bytes),
        Err(LoadError::MisalignedSegment {
            index: 0,
            align: 0x100,
            ..
        })
    ));
    for align in [0, 1] {
        put_u64(&mut bytes, 64 + 48, align);
        assert!(parse(&bytes).is_ok());
    }
}

#[test]
fn accepts_zero_fill_only_segments_without_materializing_large_buffers() {
    let mut bytes = header(2, 1);
    segment_header(
        &mut bytes,
        0,
        [6, u64::MAX, 0x1000, 0, 0, 1 << 40, 1],
        PT_LOAD,
    );
    let image = parse(&bytes).unwrap();
    assert!(image.segments[0].file_bytes.is_empty());
    assert_eq!(image.segments[0].zero_fill_size(), Some(1 << 40));
}

#[test]
fn manually_constructed_invalid_segment_metadata_cannot_underflow_zero_fill() {
    let segment = LoadSegment {
        vaddr: 0,
        mem_size: 0,
        file_offset: 0,
        file_bytes: &[1],
        flags: SegmentFlags::default(),
        align: 1,
    };
    assert_eq!(segment.zero_fill_size(), None);
}

#[test]
fn ignores_undefined_null_header_members() {
    let mut bytes = header(2, 1);
    segment_header(
        &mut bytes,
        0,
        [
            u32::MAX as u64,
            u64::MAX,
            u64::MAX,
            u64::MAX,
            u64::MAX,
            u64::MAX,
            3,
        ],
        0,
    );
    let image = parse(&bytes).unwrap();
    assert!(image.segments.is_empty());
    assert_eq!(image.program_headers[0].file_offset, u64::MAX);
    assert!(image.program_headers[0].file_bytes.is_empty());
}

#[test]
fn validates_file_ranges_of_non_loadable_headers_too() {
    let mut bytes = header(2, 1);
    segment_header(&mut bytes, 0, [0, 0x1000, 0, 0, 32, 0, 0], PT_DYNAMIC);
    assert!(matches!(
        parse(&bytes),
        Err(LoadError::Truncated {
            context: "program segment",
            ..
        })
    ));
}

#[test]
fn has_no_section_table_dependency_for_program_loading_metadata() {
    let mut bytes = executable();
    put_u64(&mut bytes, 40, u64::MAX);
    put_u16(&mut bytes, 58, u16::MAX);
    put_u16(&mut bytes, 60, u16::MAX);
    put_u16(&mut bytes, 62, u16::MAX);
    assert!(parse(&bytes).is_ok());
}

#[test]
fn ignores_reserved_identification_padding_and_retains_abi() {
    let mut bytes = header(3, 0);
    bytes[7] = 255;
    bytes[8] = 255;
    bytes[9..16].fill(0xa5);
    let image = parse(&bytes).unwrap();
    assert_eq!((image.os_abi, image.abi_version), (255, 255));
    assert!(image.program_headers.is_empty());
}

#[test]
fn hostile_header_field_mutations_do_not_panic() {
    let baseline = executable();
    // Reproducible mutations cover every byte, not just fixtures aimed at known
    // errors. No platform RNG or proprietary input is needed to rerun the test.
    for index in 0..baseline.len() {
        for value in [0, 1, 0x7f, 0x80, 0xff] {
            let mut bytes = baseline.clone();
            bytes[index] = value;
            let result = std::panic::catch_unwind(|| parse(&bytes));
            assert!(
                result.is_ok(),
                "parser panicked at byte {index}, value {value:#x}"
            );
        }
    }
}

#[test]
fn arbitrary_short_inputs_never_panic() {
    let mut state = 0x8a5cd789635d2dffu64;
    for length in 0..512 {
        let bytes: Vec<u8> = (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        assert!(std::panic::catch_unwind(|| parse(&bytes)).is_ok());
    }
}
