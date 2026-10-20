// SPDX-License-Identifier: GPL-2.0-or-later

use super::*;

const FIRST_PAYLOAD: usize = 0x300;
const SECOND_PAYLOAD: usize = 0x380;
const CONTAINER_LENGTH: usize = SECOND_PAYLOAD + 8;

pub(super) fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn elf_offset(bytes: &[u8]) -> usize {
    SELF_HEADER_SIZE as usize + usize::from(read_u16(bytes, 24)) * SELF_SEGMENT_SIZE
}

pub(super) fn ph_offset(bytes: &[u8], index: usize) -> usize {
    elf_offset(bytes) + ELF_HEADER_SIZE + index * PROGRAM_HEADER_SIZE
}

pub(super) fn write_ph(bytes: &mut [u8], at: usize, kind: u32, fields: [u64; 7]) {
    put_u32(bytes, at, kind);
    put_u32(bytes, at + 4, u32::try_from(fields[0]).unwrap());
    for (index, value) in fields[1..].iter().enumerate() {
        put_u64(bytes, at + 8 + index * 8, *value);
    }
}

pub(super) fn self_fixture(descriptor_count: u16) -> Vec<u8> {
    assert!((2..=5).contains(&descriptor_count));
    let mut bytes = vec![0; CONTAINER_LENGTH];
    bytes[..12].copy_from_slice(&[0x4f, 0x15, 0x3d, 0x1d, 0, 1, 1, 0x12, 1, 1, 0, 0]);
    put_u16(&mut bytes, 12, 0x200);
    put_u16(&mut bytes, 14, 0x20);
    put_u64(&mut bytes, 16, CONTAINER_LENGTH as u64);
    put_u16(&mut bytes, 24, descriptor_count);
    put_u16(&mut bytes, 26, 0x22);
    for (index, physical, size) in [(0, FIRST_PAYLOAD, 16), (1, SECOND_PAYLOAD, 8)] {
        let at = SELF_HEADER_SIZE as usize + index * SELF_SEGMENT_SIZE;
        put_u64(&mut bytes, at, 0x800 | ((index as u64) << 20));
        put_u64(&mut bytes, at + 8, physical as u64);
        put_u64(&mut bytes, at + 16, size);
        put_u64(&mut bytes, at + 24, size);
    }
    // Extra descriptors model integrity metadata without pretending to decode it.
    for index in 2..usize::from(descriptor_count) {
        let at = SELF_HEADER_SIZE as usize + index * SELF_SEGMENT_SIZE;
        put_u64(&mut bytes, at, 0x10000);
        put_u64(&mut bytes, at + 8, 0x240 + index as u64 * 16);
        put_u64(&mut bytes, at + 16, 16);
        put_u64(&mut bytes, at + 24, 16);
    }

    let elf = elf_offset(&bytes);
    bytes[elf..elf + 9].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 9, 2]);
    put_u16(&mut bytes, elf + 16, 0xfe10);
    put_u16(&mut bytes, elf + 18, 62);
    put_u32(&mut bytes, elf + 20, 1);
    put_u64(&mut bytes, elf + 24, 0x401000);
    put_u64(&mut bytes, elf + 32, ELF_HEADER_SIZE as u64);
    put_u16(&mut bytes, elf + 52, ELF_HEADER_SIZE as u16);
    put_u16(&mut bytes, elf + 54, PROGRAM_HEADER_SIZE as u16);
    put_u16(&mut bytes, elf + 56, 3);
    write_ph(
        &mut bytes,
        elf + ELF_HEADER_SIZE,
        PT_LOAD,
        [5, 0x1000, 0x401000, 0, 16, 32, 0x1000],
    );
    write_ph(
        &mut bytes,
        elf + ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE,
        PT_LOAD,
        [6, 0x2000, 0x402000, 0, 8, 32, 0x1000],
    );
    write_ph(
        &mut bytes,
        elf + ELF_HEADER_SIZE + 2 * PROGRAM_HEADER_SIZE,
        2,
        [6, 0x1008, 0x401008, 0, 8, 8, 8],
    );
    for index in 0..16 {
        bytes[FIRST_PAYLOAD + index] = u8::try_from(index + 1).unwrap();
    }
    bytes[SECOND_PAYLOAD..SECOND_PAYLOAD + 8]
        .copy_from_slice(&[0xb1, 0xb2, 0xb3, 0xb4, 1, 2, 3, 4]);
    bytes
}

#[test]
fn inspects_only_headers_without_requiring_payloads_in_memory() {
    let bytes = self_fixture(2);
    let headers_end = elf_offset(&bytes) + ELF_HEADER_SIZE + 3 * PROGRAM_HEADER_SIZE;
    let metadata = inspect_executable(&bytes[..headers_end], bytes.len() as u64).unwrap();
    assert_eq!(metadata.format, ExecutableFormat::SelfContainer);
    assert_eq!(metadata.entry, 0x401000);
    assert_eq!(metadata.elf_type, ElfType::SceDynamicExecutable);
    assert_eq!((metadata.os_abi, metadata.abi_version), (9, 2));
    assert_eq!(metadata.program_headers.len(), 3);
    let container = metadata.self_metadata.unwrap();
    assert_eq!(container.elf_offset, 96);
    assert_eq!(container.segments.len(), 2);
    assert!(container.segments[0].is_payload());
    assert!(!container.segments[0].is_encrypted());
    assert!(!container.segments[0].is_compressed());
    assert_eq!(container.segments[1].program_header_index(), Some(1));
}

#[test]
fn translates_each_self_payload_instead_of_slicing_at_the_elf_header() {
    let bytes = self_fixture(2);
    let image = parse_executable(&bytes).unwrap();
    assert_eq!(image.elf.segments.len(), 2);
    assert_eq!(image.elf.segments[0].file_offset, 0x1000);
    assert_eq!(
        image.elf.segments[0].file_bytes,
        &bytes[FIRST_PAYLOAD..FIRST_PAYLOAD + 16]
    );
    assert_eq!(
        image.elf.segments[0].file_bytes.as_ptr(),
        bytes[FIRST_PAYLOAD..].as_ptr()
    );
    assert_eq!(
        image.elf.segments[1].file_bytes.as_ptr(),
        bytes[SECOND_PAYLOAD..].as_ptr()
    );
    assert_eq!(image.elf.segments[0].zero_fill_size(), Some(16));
    let dynamic = image.elf.program_headers[2];
    assert_eq!(dynamic.kind, 2);
    assert_eq!(
        dynamic.file_bytes.as_ptr(),
        bytes[FIRST_PAYLOAD + 8..].as_ptr()
    );
    assert_eq!(dynamic.file_bytes, &[9, 10, 11, 12, 13, 14, 15, 16]);
    // The input does not even contain the declared logical ELF payload offsets.
    assert!(bytes.len() < 0x1000);
}

#[test]
fn derives_elf_offset_from_descriptor_count() {
    for count in [2, 3, 5] {
        let bytes = self_fixture(count);
        let image = parse_executable(&bytes).unwrap();
        let container = image.metadata.self_metadata.unwrap();
        assert_eq!(container.elf_offset, 32 + u64::from(count) * 32);
        assert_eq!(container.segments.len(), usize::from(count));
        if count > 2 {
            assert_eq!(container.segments[2].program_header_index(), None);
        }
    }
}

#[test]
fn accepts_the_corroborated_alternate_identification_variant() {
    let mut bytes = self_fixture(2);
    bytes[..4].copy_from_slice(&ALTERNATE_SELF_MAGIC);
    bytes[4] = 0x10;
    bytes[11] = 0x10;
    put_u16(&mut bytes, 26, 0x32);
    assert!(parse_executable(&bytes).is_ok());
    assert_eq!(parse(&bytes), Err(LoadError::UnsupportedSelf));
    put_u16(&mut bytes, 26, 0x22);
    assert!(matches!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(LoadError::InvalidSelf(
            SelfError::UnsupportedIdentification { .. }
        ))
    ));
}

#[test]
fn reports_an_incomplete_prefix_without_demanding_full_file_contents() {
    let bytes = self_fixture(2);
    let end = elf_offset(&bytes) + ELF_HEADER_SIZE + 3 * PROGRAM_HEADER_SIZE;
    for length in 0..end {
        assert!(inspect_executable(&bytes[..length], bytes.len() as u64).is_err());
    }
    assert!(inspect_executable(&bytes[..end], bytes.len() as u64).is_ok());
    for length in 0..bytes.len() {
        assert!(
            parse_executable(&bytes[..length]).is_err(),
            "accepted truncated container {length}"
        );
    }
}

#[test]
fn rejects_a_prefix_longer_than_the_declared_input() {
    let bytes = self_fixture(2);
    assert_eq!(
        inspect_executable(&bytes, 64),
        Err(LoadError::PrefixLongerThanFile {
            prefix_len: bytes.len() as u64,
            file_len: 64
        })
    );
}

#[test]
fn validates_self_descriptor_table_length() {
    let mut bytes = self_fixture(2);
    put_u16(&mut bytes, 24, u16::MAX);
    assert!(matches!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(LoadError::Truncated {
            context: "SELF descriptor table",
            ..
        })
    ));
}

#[test]
fn validates_header_metadata_and_declared_file_boundaries() {
    let mut bytes = self_fixture(2);
    let headers_end = elf_offset(&bytes) + ELF_HEADER_SIZE + 3 * PROGRAM_HEADER_SIZE;
    put_u16(&mut bytes, 12, headers_end as u16 - 1);
    assert!(matches!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(LoadError::InvalidSelf(SelfError::HeaderTooSmall { .. }))
    ));
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 16, 0x21f);
    assert!(matches!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(LoadError::InvalidSelf(SelfError::InvalidFileSize { .. }))
    ));
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 16, CONTAINER_LENGTH as u64 + 1);
    assert!(matches!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(LoadError::Truncated {
            context: "SELF declared file",
            ..
        })
    ));
}

#[test]
fn prevents_payload_descriptors_pointing_into_headers_or_metadata() {
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 32 + 8, 0x21f);
    assert!(matches!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(LoadError::InvalidSelf(SelfError::SegmentOverlapsHeaders {
            segment_index: 0,
            ..
        }))
    ));
}

#[test]
fn checks_descriptor_file_range_overflow_and_truncation() {
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 32 + 8, u64::MAX - 1);
    assert!(matches!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(LoadError::RangeOverflow {
            context: "SELF segment",
            ..
        })
    ));
    put_u64(&mut bytes, 32 + 8, CONTAINER_LENGTH as u64 - 1);
    assert!(matches!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(LoadError::Truncated {
            context: "SELF segment",
            ..
        })
    ));
}

#[test]
fn rejects_descriptor_indices_outside_the_embedded_program_table() {
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 32, 0x800 | (17 << 20));
    assert_eq!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(SelfError::ProgramHeaderIndexOutOfBounds {
            segment_index: 0,
            program_header_index: 17,
            program_header_count: 3,
        }
        .into())
    );
}

#[test]
fn rejects_duplicate_payload_descriptors() {
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 64, 0x800);
    assert_eq!(
        inspect_executable(&bytes, bytes.len() as u64),
        Err(SelfError::DuplicatePayload {
            program_header_index: 0
        }
        .into())
    );
}

#[test]
fn exposes_encryption_in_inspection_and_rejects_payload_access_structurally() {
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 32, 0x802);
    let metadata = inspect_executable(&bytes, bytes.len() as u64).unwrap();
    assert!(metadata.self_metadata.unwrap().segments[0].is_encrypted());
    assert_eq!(
        parse_executable(&bytes),
        Err(SelfError::EncryptedSegment { segment_index: 0 }.into())
    );
}

#[test]
fn exposes_compression_in_inspection_and_rejects_payload_access_structurally() {
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 32, 0x808);
    let metadata = inspect_executable(&bytes, bytes.len() as u64).unwrap();
    assert!(metadata.self_metadata.unwrap().segments[0].is_compressed());
    assert_eq!(
        parse_executable(&bytes),
        Err(SelfError::CompressedSegment { segment_index: 0 }.into())
    );
}

#[test]
fn checks_stored_decoded_and_elf_sizes_independently() {
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 32 + 24, 15);
    assert!(matches!(
        parse_executable(&bytes),
        Err(LoadError::InvalidSelf(SelfError::PayloadSizeMismatch {
            segment_index: 0,
            ..
        }))
    ));
    put_u64(&mut bytes, 32 + 24, 16);
    put_u64(&mut bytes, 32 + 16, 15);
    assert!(matches!(
        parse_executable(&bytes),
        Err(LoadError::InvalidSelf(SelfError::PayloadSizeMismatch {
            segment_index: 0,
            ..
        }))
    ));
}

#[test]
fn missing_load_payloads_are_not_read_from_logical_offsets() {
    let mut bytes = self_fixture(2);
    put_u64(&mut bytes, 32, 0);
    assert_eq!(
        parse_executable(&bytes),
        Err(SelfError::MissingSegment {
            program_header_index: 0,
            file_offset: 0x1000,
            size: 16,
        }
        .into())
    );
}

#[test]
fn unresolved_dynamic_and_optional_note_payloads_are_explicit_errors() {
    for kind in [2, 4, 0x61000000] {
        let mut bytes = self_fixture(2);
        let at = ph_offset(&bytes, 2);
        put_u32(&mut bytes, at, kind);
        put_u64(&mut bytes, at + 8, 0x3000);
        assert!(inspect_executable(&bytes, bytes.len() as u64).is_ok());
        assert_eq!(
            parse_executable(&bytes),
            Err(SelfError::MissingSegment {
                program_header_index: 2,
                file_offset: 0x3000,
                size: 8,
            }
            .into())
        );
    }
}

#[test]
fn resolves_known_library_version_tail_only_with_matching_type_and_size() {
    let mut bytes = self_fixture(2);
    let at = ph_offset(&bytes, 2);
    put_u32(&mut bytes, at, PT_SCE_LIBVERSION);
    put_u64(&mut bytes, at + 8, 0x3000);
    put_u64(&mut bytes, at + 32, 3);
    put_u64(&mut bytes, at + 40, 3);
    bytes.extend_from_slice(&[0xaa, 0xbb, 0xcc]);
    let image = parse_executable(&bytes).unwrap();
    assert_eq!(image.elf.program_headers[2].file_bytes, &[0xaa, 0xbb, 0xcc]);
    put_u32(&mut bytes, at, 4);
    assert!(matches!(
        parse_executable(&bytes),
        Err(LoadError::InvalidSelf(SelfError::MissingSegment {
            program_header_index: 2,
            ..
        }))
    ));
    put_u32(&mut bytes, at, PT_SCE_LIBVERSION);
    bytes.push(0xdd);
    assert!(matches!(
        parse_executable(&bytes),
        Err(LoadError::InvalidSelf(SelfError::MissingSegment {
            program_header_index: 2,
            ..
        }))
    ));
}

#[test]
fn rejects_conflicting_physical_mappings_of_overlapping_elf_ranges() {
    let mut bytes = self_fixture(2);
    let at = ph_offset(&bytes, 1);
    put_u64(&mut bytes, at + 8, 0x1000);
    assert_eq!(
        parse_executable(&bytes),
        Err(SelfError::AmbiguousSegment {
            program_header_index: 1
        }
        .into())
    );
}

#[test]
fn ignores_undefined_null_header_members_in_containers_too() {
    let mut bytes = self_fixture(2);
    let at = ph_offset(&bytes, 2);
    write_ph(
        &mut bytes,
        at,
        0,
        [0, u64::MAX, u64::MAX, 0, u64::MAX, u64::MAX, 3],
    );
    let image = parse_executable(&bytes).unwrap();
    assert!(image.elf.program_headers[2].file_bytes.is_empty());
}

#[test]
fn inspection_does_not_confuse_header_metadata_with_a_loadable_image() {
    let mut bytes = self_fixture(2);
    let at = ph_offset(&bytes, 0);
    put_u64(&mut bytes, at + 48, 3);
    assert!(inspect_executable(&bytes, bytes.len() as u64).is_ok());
    assert_eq!(
        parse_executable(&bytes),
        Err(LoadError::InvalidSegmentAlignment { index: 0, align: 3 })
    );
    put_u64(&mut bytes, at + 48, 0x1000);
    put_u64(&mut bytes, at + 40, 15);
    assert!(matches!(
        parse_executable(&bytes),
        Err(LoadError::SegmentFileSizeExceedsMemory { .. })
    ));
}

#[test]
fn supports_observed_sce_abi_two_permissionless_copy_segments() {
    let mut bytes = self_fixture(2);
    let at = ph_offset(&bytes, 0);
    put_u32(&mut bytes, at + 4, 0);
    put_u64(&mut bytes, at + 8, 0x1001);
    let image = parse_executable(&bytes).unwrap();
    let segment = image.elf.segments[0];
    assert_eq!(segment.flags.raw(), 0);
    assert_eq!(segment.align, 0x1000);
    assert_eq!(segment.file_offset, 0x1001);
    assert_eq!(segment.vaddr, 0x401000);
    assert_eq!(segment.file_bytes.as_ptr(), bytes[FIRST_PAYLOAD..].as_ptr());
    put_u64(&mut bytes, at + 48, 3);
    assert_eq!(
        parse_executable(&bytes),
        Err(LoadError::InvalidSegmentAlignment { index: 0, align: 3 })
    );
    put_u64(&mut bytes, at + 48, 0x1000);
    put_u64(&mut bytes, at + 16, u64::MAX - 1);
    assert!(matches!(
        parse_executable(&bytes),
        Err(LoadError::RangeOverflow { .. })
    ));
}

#[test]
fn copy_policy_does_not_relax_other_sce_or_standard_elf_segments() {
    let mut baseline = self_fixture(2);
    let elf = elf_offset(&baseline);
    let at = ph_offset(&baseline, 0);
    put_u32(&mut baseline, at + 4, 0);
    put_u64(&mut baseline, at + 8, 0x1001);
    for (offset, value) in [(at + 4, 1), (elf + 8, 0), (elf + 7, 0)] {
        let mut bytes = baseline.clone();
        bytes[offset] = value;
        assert!(matches!(
            parse_executable(&bytes),
            Err(LoadError::MisalignedSegment { index: 0, .. })
        ));
    }
    let mut bytes = baseline.clone();
    put_u16(&mut bytes, elf + 16, 2);
    assert!(matches!(
        parse_executable(&bytes),
        Err(LoadError::MisalignedSegment { index: 0, .. })
    ));
    // Even an SCE ABI 2 ELF outside SELF uses the strict ELF-only policy.
    let end = elf + ELF_HEADER_SIZE + 3 * PROGRAM_HEADER_SIZE;
    let mut standalone = baseline[elf..end].to_vec();
    standalone.resize(0x2008, 0);
    assert!(matches!(
        parse_executable(&standalone),
        Err(LoadError::MisalignedSegment { index: 0, .. })
    ));
}

#[test]
fn standalone_elf_inspection_accepts_header_prefix_and_checks_full_file_ranges() {
    let mut bytes = self_fixture(2);
    let elf = elf_offset(&bytes);
    let header_end = elf + ELF_HEADER_SIZE + 3 * PROGRAM_HEADER_SIZE;
    let mut standalone = bytes[elf..header_end].to_vec();
    let metadata = inspect_executable(&standalone, 0x3000).unwrap();
    assert_eq!(metadata.format, ExecutableFormat::Elf);
    assert!(metadata.self_metadata.is_none());
    assert!(matches!(
        inspect_executable(&standalone, standalone.len() as u64),
        Err(LoadError::Truncated {
            context: "program segment",
            ..
        })
    ));
    standalone.resize(0x2008, 0);
    standalone[0x1000..0x1010].copy_from_slice(&bytes[FIRST_PAYLOAD..FIRST_PAYLOAD + 16]);
    standalone[0x2000..0x2008].copy_from_slice(&bytes[SECOND_PAYLOAD..SECOND_PAYLOAD + 8]);
    let parsed = parse_executable(&standalone).unwrap();
    assert_eq!(parsed.elf, parse(&standalone).unwrap());
    bytes.fill(0); // The standalone test data has its own ownership.
}

#[test]
fn malformed_container_mutations_do_not_panic_or_allocate_guest_memory() {
    let baseline = self_fixture(3);
    for index in 0..baseline.len() {
        for value in [0, 1, 0x7f, 0xff] {
            let mut bytes = baseline.clone();
            bytes[index] = value;
            let inspected =
                std::panic::catch_unwind(|| inspect_executable(&bytes, bytes.len() as u64));
            let parsed = std::panic::catch_unwind(|| parse_executable(&bytes));
            assert!(
                inspected.is_ok(),
                "inspection panicked at {index}, {value:#x}"
            );
            assert!(
                parsed.is_ok(),
                "translation panicked at {index}, {value:#x}"
            );
        }
    }
}
