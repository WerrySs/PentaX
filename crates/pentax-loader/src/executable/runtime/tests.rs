// SPDX-License-Identifier: GPL-2.0-or-later

use super::super::tests::{
    elf_offset, ph_offset, put_u16, put_u32, put_u64, self_fixture, write_ph,
};
use super::*;
use crate::{SelfError, parse_executable};

fn container() -> Vec<u8> {
    let mut bytes = self_fixture(2);
    let at = ph_offset(&bytes, 2);
    write_ph(
        &mut bytes,
        at,
        PT_DYNAMIC,
        [6, 0x1000, 0x401000, 0, 16, 16, 8],
    );
    bytes[0x300..0x310].fill(0); // One bounded DT_NULL record.
    let elf = elf_offset(&bytes);
    put_u16(&mut bytes, elf + 56, 4);
    let note = ph_offset(&bytes, 3);
    write_ph(&mut bytes, note, 4, [0, 0x5000, 0, 0, 8, 0, 4]);
    bytes
}

fn dynamic_bytes(entries: &[(i64, u64)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for &(tag, value) in entries {
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&[0; 16]);
    bytes
}

#[test]
fn initialized_reads_reject_source_bytes_beyond_the_declared_memory_image() {
    let mut bytes = elf_with_dynamic(&[], false);
    put_u64(&mut bytes, 64 + 40, 0x100);
    let view = open_executable(&bytes).unwrap();
    let mut output = [0xa5; 16];
    assert!(matches!(
        view.read_initialized_vaddr(0x8100, &mut output),
        Err(LoadError::SegmentFileSizeExceedsMemory { index: 0, .. })
    ));
    assert_eq!(output, [0xa5; 16]);
    assert!(matches!(
        view.runtime_plan(),
        Err(LoadError::SegmentFileSizeExceedsMemory { index: 0, .. })
    ));
}

#[test]
fn missing_plt_format_identifies_the_actual_standard_or_sce_table_tag() {
    for (table, size, format) in [(23, 2, 20), (0x61000029, 0x6100002d, 0x6100002b)] {
        assert_eq!(
            parse_dynamic(&dynamic_bytes(&[(table, 0), (size, 24)])),
            Err(DynamicError::MissingCompanion {
                tag: table,
                required: format,
            }
            .into())
        );
    }
}

fn elf_with_dynamic(entries: &[(i64, u64)], dynlib: bool) -> Vec<u8> {
    let mut bytes = vec![0; 0x400];
    bytes[..9].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 9, 2]);
    put_u16(&mut bytes, 16, 0xfe10);
    put_u16(&mut bytes, 18, 62);
    put_u32(&mut bytes, 20, 1);
    put_u64(&mut bytes, 24, 0x8000);
    put_u64(&mut bytes, 32, 64);
    put_u16(&mut bytes, 52, 64);
    put_u16(&mut bytes, 54, 56);
    put_u16(&mut bytes, 56, if dynlib { 3 } else { 2 });
    write_ph(
        &mut bytes,
        64,
        PT_LOAD,
        [6, 0x100, 0x8000, 0, 0x200, 0x220, 1],
    );
    let dynamic = dynamic_bytes(entries);
    assert!(dynamic.len() <= 0x100);
    write_ph(
        &mut bytes,
        120,
        PT_DYNAMIC,
        [
            6,
            0x100,
            0x8000,
            0,
            dynamic.len() as u64,
            dynamic.len() as u64,
            8,
        ],
    );
    bytes[0x100..0x100 + dynamic.len()].copy_from_slice(&dynamic);
    if dynlib {
        write_ph(
            &mut bytes,
            176,
            PT_SCE_DYNLIBDATA,
            [0, 0x300, 0, 0, 0x100, 0, 0],
        );
    }
    bytes
}

#[test]
fn optional_unmapped_notes_are_explicit_while_strict_parsing_still_fails() {
    let bytes = container();
    assert!(matches!(
        parse_executable(&bytes),
        Err(LoadError::InvalidSelf(SelfError::MissingSegment {
            program_header_index: 3,
            ..
        }))
    ));
    let view = open_executable(&bytes).unwrap();
    let plan = view.runtime_plan().unwrap();
    assert_eq!(plan.segments.len(), 2);
    assert_eq!(
        plan.dispositions[3].purpose,
        HeaderPurpose::MetadataOnly(MetadataPurpose::AuxiliaryNote)
    );
    assert!(matches!(
        view.program_bytes(3),
        Err(LoadError::InvalidSelf(SelfError::MissingSegment { .. }))
    ));
    assert!(plan.dynamic.unwrap().entries.is_empty());
}

#[test]
fn missing_required_loads_and_dynamic_payloads_still_fail() {
    let mut bytes = container();
    put_u64(&mut bytes, 32, 0);
    assert!(matches!(
        open_executable(&bytes).unwrap().runtime_plan(),
        Err(LoadError::InvalidSelf(SelfError::MissingSegment {
            program_header_index: 0,
            ..
        }))
    ));
    let mut bytes = container();
    let at = ph_offset(&bytes, 2);
    put_u64(&mut bytes, at + 8, 0x5000);
    assert!(matches!(
        open_executable(&bytes).unwrap().runtime_plan(),
        Err(LoadError::InvalidSelf(SelfError::MissingSegment {
            program_header_index: 2,
            ..
        }))
    ));
}

#[test]
fn unknown_headers_and_invalid_indices_have_structured_errors() {
    let mut bytes = container();
    let at = ph_offset(&bytes, 3);
    put_u32(&mut bytes, at, 0x60001234);
    let view = open_executable(&bytes).unwrap();
    assert_eq!(
        view.runtime_plan(),
        Err(RuntimeError::UnsupportedHeader {
            index: 3,
            kind: 0x60001234
        }
        .into())
    );
    assert_eq!(
        view.program_bytes(4),
        Err(RuntimeError::ProgramHeaderIndex { index: 4, count: 4 }.into())
    );
}

#[test]
fn tls_and_process_parameter_views_require_actual_initialized_bytes() {
    let mut bytes = container();
    let at = ph_offset(&bytes, 3);
    write_ph(&mut bytes, at, PT_TLS, [4, 0x1000, 0x401000, 0, 16, 32, 8]);
    let plan = open_executable(&bytes).unwrap().runtime_plan().unwrap();
    assert_eq!(plan.tls.unwrap().mem_size, 32);
    write_ph(
        &mut bytes,
        at,
        PT_SCE_PROCPARAM,
        [4, 0x1000, 0x401000, 0, 16, 16, 8],
    );
    assert!(
        open_executable(&bytes)
            .unwrap()
            .runtime_plan()
            .unwrap()
            .process_parameters
            .is_some()
    );
    put_u64(&mut bytes, at + 16, 0x600000);
    assert!(matches!(
        open_executable(&bytes).unwrap().runtime_plan(),
        Err(LoadError::Runtime(
            RuntimeError::UnmappedVirtualRange { .. }
        ))
    ));
}

fn elf_with_parameters(kinds: &[u32]) -> Vec<u8> {
    assert!(kinds.len() <= 2);
    let mut bytes = elf_with_dynamic(&[], false);
    bytes.resize(0x600, 0);
    // Leave room for all headers before the original initialized load bytes.
    put_u64(&mut bytes, 64 + 8, 0x200);
    put_u64(&mut bytes, 120 + 8, 0x200);
    put_u16(&mut bytes, 56, 2 + kinds.len() as u16);
    for (index, &kind) in kinds.iter().enumerate() {
        write_ph(
            &mut bytes,
            176 + index * 56,
            kind,
            [4, 0x300, 0x8100, 0, 8, 8, 8],
        );
    }
    bytes[0x300..0x308].fill(0xa5);
    bytes
}

#[test]
fn module_and_process_parameters_have_distinct_ranges_without_abi_or_content_guesses() {
    for (os_abi, abi_version) in [(0, 0), (9, 2)] {
        let mut bytes = elf_with_parameters(&[PT_SCE_PROCPARAM, PT_SCE_MODULE_PARAM]);
        bytes[7] = os_abi;
        bytes[8] = abi_version;
        write_ph(
            &mut bytes,
            232,
            PT_SCE_MODULE_PARAM,
            [4, 0x310, 0x8110, 0, 1, 0x1000, 8],
        );
        let plan = open_executable(&bytes).unwrap().runtime_plan().unwrap();
        assert_eq!(
            plan.process_parameters,
            Some(VirtualRange {
                address: 0x8100,
                size: 8,
            })
        );
        assert_eq!(
            plan.module_parameters,
            Some(VirtualRange {
                address: 0x8110,
                size: 1,
            })
        );
        assert_eq!(
            plan.dispositions[2].purpose,
            HeaderPurpose::ProcessParameters
        );
        assert_eq!(
            plan.dispositions[3].purpose,
            HeaderPurpose::ModuleParameters
        );
        assert_eq!(plan.segments.len(), 1);
        assert_eq!(
            plan.segments[0].file_bytes.as_ptr(),
            bytes[0x200..].as_ptr()
        );
    }
}

#[test]
fn both_parameter_roles_remain_singletons_even_when_empty() {
    for kind in [PT_SCE_PROCPARAM, PT_SCE_MODULE_PARAM] {
        for size in [0, 8] {
            let mut bytes = elf_with_parameters(&[kind, kind]);
            put_u64(&mut bytes, 176 + 32, size);
            assert_eq!(
                open_executable(&bytes).unwrap().runtime_plan(),
                Err(RuntimeError::DuplicateHeader { kind }.into())
            );
        }
    }
}

#[test]
fn module_parameters_reject_bss_gaps_page_padding_and_partial_backing() {
    for address in [0x8200, 0x8400, 0xbff8, 0x81fc] {
        let mut bytes = elf_with_parameters(&[PT_SCE_MODULE_PARAM]);
        put_u64(&mut bytes, 176 + 16, address);
        assert!(matches!(
            open_executable(&bytes).unwrap().runtime_plan(),
            Err(LoadError::Runtime(
                RuntimeError::UnmappedVirtualRange { .. }
            ))
        ));
    }
}

#[test]
fn module_parameter_virtual_range_overflow_is_checked() {
    let mut bytes = elf_with_parameters(&[PT_SCE_MODULE_PARAM]);
    put_u64(&mut bytes, 176 + 16, u64::MAX - 3);
    assert!(matches!(
        open_executable(&bytes).unwrap().runtime_plan(),
        Err(LoadError::RangeOverflow {
            context: "virtual read",
            ..
        })
    ));
}

#[test]
fn empty_parameter_ranges_keep_the_existing_process_parameter_policy() {
    for kind in [PT_SCE_PROCPARAM, PT_SCE_MODULE_PARAM] {
        let mut bytes = elf_with_parameters(&[kind]);
        write_ph(&mut bytes, 176, kind, [4, u64::MAX, u64::MAX, 0, 0, 0, 8]);
        let plan = open_executable(&bytes).unwrap().runtime_plan().unwrap();
        let expected = Some(VirtualRange {
            address: u64::MAX,
            size: 0,
        });
        if kind == PT_SCE_PROCPARAM {
            assert_eq!(plan.process_parameters, expected);
            assert_eq!(plan.module_parameters, None);
        } else {
            assert_eq!(plan.module_parameters, expected);
            assert_eq!(plan.process_parameters, None);
        }
    }
}

#[test]
fn elf_module_parameters_preserve_whole_file_range_validation() {
    let mut bytes = elf_with_parameters(&[PT_SCE_MODULE_PARAM]);
    put_u64(&mut bytes, 176 + 8, 0x9000);
    assert!(matches!(
        open_executable(&bytes),
        Err(LoadError::Truncated { .. })
    ));
    assert!(matches!(
        parse_executable(&bytes),
        Err(LoadError::Truncated { .. })
    ));
}

#[test]
fn self_module_parameters_alias_load_bytes_and_need_no_extra_mapping() {
    let mut bytes = container();
    let at = ph_offset(&bytes, 3);
    write_ph(
        &mut bytes,
        at,
        PT_SCE_MODULE_PARAM,
        [4, 0x1000, 0x401000, 0, 16, 16, 8],
    );
    let view = open_executable(&bytes).unwrap();
    let plan = view.runtime_plan().unwrap();
    assert_eq!(
        plan.module_parameters,
        Some(VirtualRange {
            address: 0x401000,
            size: 16,
        })
    );
    assert_eq!(
        plan.dispositions[3].purpose,
        HeaderPurpose::ModuleParameters
    );
    assert_eq!(plan.segments.len(), 2);
    assert_eq!(
        view.program_bytes(3).unwrap().as_ptr(),
        bytes[0x300..].as_ptr()
    );
    assert!(parse_executable(&bytes).is_ok());

    // Runtime uses the initialized virtual load range, but explicit/strict
    // payload access must still reject an unavailable separate logical range.
    put_u64(&mut bytes, at + 8, 0x5000);
    let view = open_executable(&bytes).unwrap();
    assert!(view.runtime_plan().unwrap().module_parameters.is_some());
    for result in [
        view.program_bytes(3).map(|_| ()),
        parse_executable(&bytes).map(|_| ()),
    ] {
        assert!(matches!(
            result,
            Err(LoadError::InvalidSelf(SelfError::MissingSegment {
                program_header_index: 3,
                ..
            }))
        ));
    }
}

#[test]
fn virtual_reads_distinguish_original_addresses_file_offsets_and_bss() {
    let bytes = container();
    let view = open_executable(&bytes).unwrap();
    let mut output = [0xaa; 16];
    view.read_initialized_vaddr(0x401000, &mut output).unwrap();
    assert_eq!(output, [0; 16]);
    assert!(view.read_initialized_vaddr(0x1000, &mut output).is_err());
    output.fill(0xaa);
    assert!(view.read_initialized_vaddr(0x401008, &mut output).is_err());
    assert_eq!(output, [0xaa; 16]);
    assert!(view.read_initialized_vaddr(u64::MAX, &mut output).is_err());
}

#[test]
fn reads_across_contiguous_virtual_segments_with_discontiguous_payloads() {
    let mut bytes = container();
    let at = ph_offset(&bytes, 1);
    put_u64(&mut bytes, at + 16, 0x401010);
    put_u64(&mut bytes, at + 48, 1);
    let mut output = [0; 16];
    open_executable(&bytes)
        .unwrap()
        .read_initialized_vaddr(0x401008, &mut output)
        .unwrap();
    assert_eq!(&output[..8], &[0; 8]);
    assert_eq!(&output[8..], &bytes[0x380..0x388]);
}

#[test]
fn virtual_overlaps_beginning_inside_a_read_are_not_skipped() {
    let mut bytes = container();
    let at = ph_offset(&bytes, 1);
    put_u64(&mut bytes, at + 16, 0x401008);
    put_u64(&mut bytes, at + 48, 1);
    let mut output = [0xaa; 16];
    assert!(matches!(
        open_executable(&bytes)
            .unwrap()
            .read_initialized_vaddr(0x401000, &mut output),
        Err(LoadError::Runtime(
            RuntimeError::AmbiguousVirtualRange { .. }
        ))
    ));
    assert_eq!(output, [0xaa; 16]);
}

#[test]
fn relro_can_cover_the_validated_final_guest_page_padding() {
    let mut bytes = container();
    let at = ph_offset(&bytes, 3);
    write_ph(
        &mut bytes,
        at,
        PT_GNU_RELRO,
        [4, 0x1000, 0x401000, 0, 16, 0x1000, 1],
    );
    assert_eq!(
        open_executable(&bytes)
            .unwrap()
            .runtime_plan()
            .unwrap()
            .relro[0]
            .size,
        0x1000
    );
    put_u64(&mut bytes, at + 40, 0x4000);
    assert!(open_executable(&bytes).unwrap().runtime_plan().is_err());
}

#[test]
fn elf_alignment_cannot_invent_relro_coverage_in_an_unmapped_gap() {
    let mut bytes = elf_with_dynamic(&[], false);
    bytes.resize(0x1001, 0);
    put_u16(&mut bytes, 56, 3);
    write_ph(
        &mut bytes,
        64,
        PT_LOAD,
        [5, 0x1000, 0x1000, 0, 1, 1, 0x10000],
    );
    write_ph(&mut bytes, 176, PT_GNU_RELRO, [4, 0, 0x10000, 0, 0, 16, 1]);
    assert_eq!(
        open_executable(&bytes).unwrap().runtime_plan(),
        Err(RuntimeError::UnmappedVirtualRange {
            address: 0x10000,
            size: 16
        }
        .into())
    );
    // Both leading and trailing guest-page padding really are mapped; logical
    // p_memsz and initialized byte coverage remain only one byte at 0x1000.
    for (address, size) in [(0, RUNTIME_PAGE_SIZE), (0x1000, 1), (0x3ff0, 16)] {
        put_u64(&mut bytes, 176 + 16, address);
        put_u64(&mut bytes, 176 + 40, size);
        let view = open_executable(&bytes).unwrap();
        assert!(view.runtime_plan().is_ok());
        assert!(view.read_initialized_vaddr(0, &mut [0]).is_err());
    }
    // An empty load segment maps no page, even if its address is unaligned.
    put_u64(&mut bytes, 64 + 32, 0);
    put_u64(&mut bytes, 64 + 40, 0);
    assert!(open_executable(&bytes).unwrap().runtime_plan().is_err());
}

#[test]
fn standard_dynamic_regions_use_guest_addresses() {
    let bytes = elf_with_dynamic(
        &[
            (5, 0x8100),
            (10, 8),
            (6, 0x8120),
            (11, 24),
            (7, 0x8140),
            (8, 24),
            (9, 24),
        ],
        false,
    );
    let plan = open_executable(&bytes).unwrap().runtime_plan().unwrap();
    let dynamic = plan.dynamic.unwrap();
    assert_eq!(
        dynamic.strings.unwrap().address,
        DynamicAddress::GuestVirtual(0x8100)
    );
    assert_eq!(dynamic.symbols.unwrap().size, None);
    assert_eq!(dynamic.rela.unwrap().size, Some(24));
}

#[test]
fn sce_dynamic_offsets_require_and_use_dynlib_backing() {
    let tags = [
        (0x61000035, 0x20),
        (0x61000037, 8),
        (0x61000039, 0x40),
        (0x6100003f, 24),
        (0x6100003b, 24),
    ];
    let bytes = elf_with_dynamic(&tags, true);
    let plan = open_executable(&bytes).unwrap().runtime_plan().unwrap();
    assert_eq!(
        plan.dynamic.unwrap().strings.unwrap().address,
        DynamicAddress::DynlibOffset(0x20)
    );
    let missing = elf_with_dynamic(&tags, false);
    assert_eq!(
        open_executable(&missing).unwrap().runtime_plan(),
        Err(RuntimeError::MissingDynlibData.into())
    );
    let bad = elf_with_dynamic(&[(0x61000035, 0xfc), (0x61000037, 8)], true);
    assert!(matches!(
        open_executable(&bad).unwrap().runtime_plan(),
        Err(LoadError::Truncated {
            context: "SCE dynamic table",
            ..
        })
    ));
}

#[test]
fn rejects_standard_dynamic_addresses_in_file_offsets_or_bss() {
    for address in [0x200, 0x8200, u64::MAX - 3] {
        let bytes = elf_with_dynamic(&[(5, address), (10, 8)], false);
        assert!(open_executable(&bytes).unwrap().runtime_plan().is_err());
    }
}

#[test]
fn dynamic_records_are_bounded_and_companions_are_checked() {
    assert!(parse_dynamic(&[]).is_err());
    assert!(parse_dynamic(&[0; 15]).is_err());
    let mut no_null = [0; 16];
    no_null[0] = 1;
    assert_eq!(
        parse_dynamic(&no_null),
        Err(DynamicError::MissingTerminator.into())
    );
    for tags in [
        vec![(5, 0x100), (5, 0x100), (10, 1)],
        vec![(5, 0x100)],
        vec![(6, 0x100), (11, 16)],
        vec![(7, 0x100), (8, 25), (9, 24)],
        vec![(23, 0x100), (2, 24), (20, 17)],
    ] {
        assert!(parse_dynamic(&dynamic_bytes(&tags)).is_err());
    }
}

#[test]
fn rejects_conflicting_domains_and_retains_unknown_or_repeated_import_tags() {
    for tags in [
        vec![(5, 0x100), (0x61000035, 1), (10, 8)],
        vec![(5, 0x100), (10, 8), (0x61000037, 8)],
    ] {
        assert!(matches!(
            parse_dynamic(&dynamic_bytes(&tags)),
            Err(LoadError::Runtime(RuntimeError::Dynamic(
                DynamicError::MixedAddressDomains { .. }
            )))
        ));
    }
    let dynamic = parse_dynamic(&dynamic_bytes(&[(1, 1), (1, 2), (0x61000045, 0x1234)])).unwrap();
    assert_eq!(dynamic.entries.len(), 3);
}

#[test]
fn malformed_runtime_mutations_return_errors_without_panics() {
    let baseline = container();
    for index in 0..baseline.len() {
        let mut bytes = baseline.clone();
        bytes[index] = 0xff;
        assert!(
            std::panic::catch_unwind(
                || open_executable(&bytes).and_then(|view| view.runtime_plan())
            )
            .is_ok(),
            "byte {index}"
        );
    }
}
