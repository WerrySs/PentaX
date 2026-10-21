// SPDX-License-Identifier: GPL-2.0-only
//! Original qualified consumers of four stateless AudioOut2 helpers.

use super::*;

#[test]
fn audio_out2_exact_bindings_append_eight_rows_and_keep_all_old_sites_and_data() {
    let provider = factory();
    assert_eq!(
        (
            provider.registration_count(),
            MODULES.len(),
            LIBRARIES.len()
        ),
        (1326, 69, 71)
    );
    assert_eq!(
        (
            provider.code_bytes().len(),
            provider.metadata_mapped_bytes()
        ),
        (0x2a70, 4 * PAGE_SIZE)
    );
    assert_eq!(
        &MODULES[66..68],
        &[("AudioOut", (1, 1)), ("libSceAudioOut", (1, 1))]
    );
    assert_eq!(
        &LIBRARIES[67..69],
        &[("AudioOut2", 1), ("libSceAudioOut2", 1)]
    );
    assert_eq!((MAX_REGISTRATIONS, CODE_MAPPED_BYTES), (2048, 0x10000));
    assert_eq!(
        (provider.objects()[0].symbol, provider.objects()[0].rva),
        (1327, 0x10000)
    );
    assert_eq!(
        (provider.objects()[1].symbol, provider.objects()[1].rva),
        (1328, 0x10008)
    );
    assert_eq!(PROGRAM_NAME_BUFFER_RVA, 0x10010);
    for (index, marker) in provider.markers()[..1283].iter().enumerate() {
        assert_eq!(
            (marker.symbol, marker.service_id, marker.rva),
            (
                index + 1,
                0x5058_4b01 + index as u32,
                0x100 + index as u64 * 8
            )
        );
        assert_eq!(
            &provider.code_bytes()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
    }
    for (start, module, library) in [
        (1283, b"AudioOut".as_slice(), b"AudioOut2".as_slice()),
        (
            1287,
            b"libSceAudioOut".as_slice(),
            b"libSceAudioOut2".as_slice(),
        ),
    ] {
        let rows = &Service::registrations()[start..start + 4];
        let bytes = consumer(module, library, rows).build().unwrap();
        let linked = link(&bytes, &provider).unwrap();
        assert_eq!(linked.patch_count(), 5);
        let imports = linked
            .patches()
            .iter()
            .filter(|p| p.consumer().index() == 0);
        assert_eq!(imports.clone().count(), 4);
        for (offset, patch) in imports.enumerate() {
            let index = start + offset;
            assert_eq!(patch.value(), PROVIDER_BIAS + 0x100 + index as u64 * 8);
            assert!(
                matches!(patch.resolution(), Resolution::Definition(reference) if reference.module.index() == 1 && reference.symbol == index + 1)
            );
        }
    }
    let view = open_executable(provider.image()).unwrap();
    let runtime = view.runtime_plan().unwrap();
    assert_eq!(runtime.segments.len(), 3);
    assert_eq!(
        (
            runtime.segments[0].flags.raw(),
            runtime.segments[1].flags.raw(),
            runtime.segments[2].flags.raw()
        ),
        (5, 6, 4)
    );
    assert!(provider.metadata_mapped_bytes() <= MAX_METADATA_BYTES);
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn audio_out2_refuses_unproven_scopes_versions_types_and_speaker_array_creation() {
    let provider = factory();
    for row in &Service::registrations()[1283..1287] {
        for (module, library) in [
            (b"AudioOut".as_slice(), b"libSceAudioOut2".as_slice()),
            (b"libSceAudioOut".as_slice(), b"AudioOut2".as_slice()),
            (b"AudioOut2".as_slice(), b"AudioOut2".as_slice()),
        ] {
            let bytes = consumer(module, library, &[*row]).build().unwrap();
            assert!(matches!(
                link(&bytes, &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
            let mut original = consumer(b"AudioOut", b"AudioOut2", &[*row]);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut object = consumer(b"AudioOut", b"AudioOut2", &[*row]);
        object.imports[0].symbol_type = 1;
        object.imports[0].relocation = 6;
        assert!(matches!(
            link(&object.build().unwrap(), &provider),
            Err(SymbolicError::UnsupportedSymbol {
                reason: UnsupportedSymbol::TypeMismatch,
                ..
            })
        ));
    }
    let mut create = consumer(
        b"AudioOut",
        b"AudioOut2",
        &Service::registrations()[1283..1284],
    );
    create.imports[0].nid = *b"+k91hoTuoA8";
    assert!(matches!(
        link(&create.build().unwrap(), &provider),
        Err(SymbolicError::MissingProvider { .. })
    ));
}

#[test]
fn audio_out2_lifecycle_exact_pairs_append_without_moving_any_old_marker() {
    let provider = factory();
    assert_eq!(provider.registration_count(), 1326);
    for (index, marker) in provider.markers()[..1291].iter().enumerate() {
        assert_eq!(
            (marker.symbol, marker.service_id, marker.rva),
            (
                index + 1,
                0x5058_4b01 + index as u32,
                0x100 + index as u64 * 8
            )
        );
        assert_eq!(
            &provider.code_bytes()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
    }
    for (start, module, library) in [
        (1291, b"AudioOut".as_slice(), b"AudioOut2".as_slice()),
        (
            1302,
            b"libSceAudioOut".as_slice(),
            b"libSceAudioOut2".as_slice(),
        ),
    ] {
        for (chunk, rows) in Service::registrations()[start..start + 11]
            .chunks(8)
            .enumerate()
        {
            let bytes = consumer(module, library, rows).build().unwrap();
            let linked = link(&bytes, &provider).unwrap();
            assert_eq!(linked.patch_count(), rows.len() + 1);
            let imports = linked
                .patches()
                .iter()
                .filter(|patch| patch.consumer().index() == 0);
            assert_eq!(imports.clone().count(), rows.len());
            for (offset, patch) in imports.enumerate() {
                let index = start + chunk * 8 + offset;
                assert_eq!(patch.value(), PROVIDER_BIAS + 0x100 + index as u64 * 8);
                assert!(
                    matches!(patch.resolution(), Resolution::Definition(reference)
                        if reference.module.index() == 1 && reference.symbol == index + 1)
                );
            }
        }
    }
}

#[test]
fn audio_out2_lifecycle_does_not_generalize_scopes_versions_or_object_types() {
    let provider = factory();
    for row in &Service::registrations()[1291..1302] {
        for (module, library) in [
            (b"AudioOut".as_slice(), b"libSceAudioOut2".as_slice()),
            (b"libSceAudioOut".as_slice(), b"AudioOut2".as_slice()),
            (b"AudioOut2".as_slice(), b"AudioOut2".as_slice()),
        ] {
            let bytes = consumer(module, library, &[*row]).build().unwrap();
            assert!(matches!(
                link(&bytes, &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
            let mut original = consumer(b"AudioOut", b"AudioOut2", &[*row]);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut object = consumer(b"AudioOut", b"AudioOut2", &[*row]);
        object.imports[0].symbol_type = 1;
        object.imports[0].relocation = 6;
        assert!(matches!(
            link(&object.build().unwrap(), &provider),
            Err(SymbolicError::UnsupportedSymbol {
                reason: UnsupportedSymbol::TypeMismatch,
                ..
            })
        ));
    }
}
