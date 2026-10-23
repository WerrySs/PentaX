// SPDX-License-Identifier: GPL-2.0-only
//! Original full decoder and physical-layout consumer for direct DCB exports.
use super::*;
use pentax_linker::{Limits, SceLimits, SymbolBinding, SymbolType, inspect_sce};

#[test]
fn direct_dcb_exports_keep_all_old_stubs_and_decode_exact_scopes_inside_one_page() {
    let provider = build_graphics_provider(PAGE_SIZE).unwrap();
    let image = provider.image();
    assert_eq!(image.len(), 16384);
    assert_eq!(provider.function_count(), 283);
    assert_eq!(
        (STRINGS_AT, NAME_BYTES, DYNAMIC_AT, NIDS_AT, SYMBOLS_AT),
        (176, 68, 4720, 5040, 9568)
    );
    assert_eq!(SYMBOLS_AT + 284 * 24, image.len());
    assert_eq!(image[STRINGS_AT], 0);
    assert_eq!(image[SYMBOLS_AT - 1], 0);
    let view = pentax_loader::open_executable(image).unwrap();
    let metadata = inspect_sce(&view, &Limits::DEFAULT, &SceLimits::DEFAULT)
        .unwrap()
        .into_metadata()
        .unwrap();
    assert_eq!(metadata.link().symbols().len(), 284);
    let descriptors = metadata.descriptors().decoded().unwrap();
    for (index, name) in NAMES.iter().enumerate() {
        assert_eq!(
            descriptors.module_info()[index].name().as_bytes(),
            name.as_bytes()
        );
        assert_eq!(
            descriptors.export_libraries()[index].name().as_bytes(),
            name.as_bytes()
        );
        assert_eq!(
            (
                descriptors.module_info()[index].version_major(),
                descriptors.module_info()[index].version_minor()
            ),
            (1, 1)
        );
        assert_eq!(descriptors.export_libraries()[index].version(), 1);
        let start = STRINGS_AT + NAME_OFFSETS[index];
        assert_eq!(&image[start..start + name.len()], name.as_bytes());
        assert_eq!(image[start + name.len()], 0);
    }
    let mut intervals = vec![
        (0, 176),
        (STRINGS_AT, STRINGS_AT + NAME_BYTES),
        (DYNAMIC_AT, NIDS_AT),
        (NIDS_AT, SYMBOLS_AT),
        (SYMBOLS_AT, image.len()),
    ];
    for (ordinal, marker) in provider.markers().enumerate() {
        assert_eq!(marker.id(), 0x5058_4701 + ordinal as u32);
        let expected = if ordinal < 279 {
            0x100 + ordinal * 16
        } else {
            0x108 + (ordinal - 279) * 16
        };
        assert_eq!(marker.rva(), expected as u64);
        assert_eq!(&image[expected..expected + 8], &marker.bytes());
        assert_eq!(
            metadata.link().symbols()[ordinal + 1].value,
            expected as u64
        );
        intervals.push((expected, expected + 8));
    }
    intervals.sort_unstable();
    assert!(intervals.windows(2).all(|pair| pair[0].1 <= pair[1].0));
    for (ordinal, nid, scope, suffix) in [
        (279, "UglJIZjGssM", 0, "#B#B"),
        (280, "UglJIZjGssM", 6, "#H#H"),
        (281, "AhGvpITrf4M", 0, "#B#B"),
        (282, "AhGvpITrf4M", 6, "#H#H"),
    ] {
        assert_eq!(ENTRIES[ordinal], (nid, Function::Dcb, scope, scope));
        let symbol = &metadata.link().symbols()[ordinal + 1];
        assert_eq!(
            symbol.name.as_ref().unwrap().as_bytes(),
            format!("{nid}{suffix}").as_bytes()
        );
        assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
        assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
    }
    assert!(
        !ENTRIES
            .iter()
            .any(|row| matches!(row.0, "6UzEidRZwkg" | "+T8Xo6LtFJI"))
    );
    assert!(matches!(
        build_graphics_provider(PAGE_SIZE - 1),
        Err(KernelProviderError::Budget)
    ));
}
