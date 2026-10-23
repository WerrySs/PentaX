// SPDX-License-Identifier: GPL-2.0-only
use super::*;
#[test]
fn presentation_exports_append_exact_typed_canonical_rows_without_moving_old_markers() {
    use pentax_linker::{Limits, SceLimits, SymbolBinding, SymbolType, inspect_sce};
    let provider = build_graphics_provider(PAGE_SIZE).unwrap();
    let view = pentax_loader::open_executable(provider.image()).unwrap();
    let metadata = inspect_sce(&view, &Limits::DEFAULT, &SceLimits::DEFAULT)
        .unwrap()
        .into_metadata()
        .unwrap();
    let descriptors = metadata.descriptors().decoded().unwrap();
    let module = &descriptors.module_info()[3];
    let library = &descriptors.export_libraries()[3];
    assert_eq!(module.name().as_bytes(), b"VideoOut");
    assert_eq!((module.version_major(), module.version_minor()), (1, 1));
    assert_eq!(library.name().as_bytes(), b"VideoOut");
    assert_eq!(library.version(), 1);
    assert_eq!(provider.function_count(), 283);
    assert_eq!(metadata.link().symbols().len(), 284);
    for (index, nid) in [
        "HXzjK9yI30k",
        "-Ozn0F1AFRg",
        "Xru92wHJRmg",
        "oNOQn3knW6s",
        "U2JJtSqNKZI",
        "rWUTcKdkUzQ",
        "Mt4QHHkxkOc",
        "U46NwOiJpys",
        "SbU3dwp80lQ",
        "zgXifHT9ErY",
    ]
    .iter()
    .enumerate()
    {
        let ordinal = 108 + index;
        let symbol = &metadata.link().symbols()[ordinal + 1];
        assert_eq!(
            ENTRIES.iter().filter(|(name, _, _, _)| name == nid).count(),
            if matches!(*nid, "HXzjK9yI30k" | "zgXifHT9ErY") {
                2
            } else {
                1
            }
        );
        assert_eq!((ENTRIES[ordinal].2, ENTRIES[ordinal].3), (3, 3));
        assert_eq!(
            symbol.name.as_ref().unwrap().as_bytes(),
            format!("{nid}#E#E").as_bytes()
        );
        assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
        assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
        let marker = provider.markers().nth(ordinal).unwrap();
        assert_eq!(symbol.value, 0x100 + ordinal as u64 * 16);
        assert_eq!(symbol.value, marker.rva());
        assert_eq!(
            &provider.image()[marker.rva() as usize..marker.rva() as usize + 8],
            &marker.bytes()
        );
    }
    let last_old = provider.markers().nth(107).unwrap();
    assert_eq!(
        (last_old.id(), last_old.rva(), last_old.function),
        (0x5058_476c, 0x7b0, Function::AgcDcbSetFlip)
    );
    assert!(SYMBOLS_AT + metadata.link().symbols().len() * 24 <= provider.image().len());
}

#[test]
fn video_state_exports_append_only_three_exact_canonical_markers() {
    let provider = build_graphics_provider(PAGE_SIZE).unwrap();
    for (offset, (nid, function)) in [
        ("1FZBKy8HeNU", Function::VideoGetVblankStatus),
        ("DYhhWbJSeRg", Function::VideoColorSettingsSetGamma),
        ("pv9CI5VC+R0", Function::VideoAdjustColor),
    ]
    .into_iter()
    .enumerate()
    {
        let ordinal = 222 + offset;
        assert_eq!(ENTRIES[ordinal], (nid, function, 3, 3));
        assert_eq!(ENTRIES.iter().filter(|row| row.0 == nid).count(), 2);
        let marker = provider.markers().nth(ordinal).unwrap();
        assert_eq!(marker.function, function);
        assert_eq!(marker.rva(), 0x100 + ordinal as u64 * 16);
        assert_eq!(marker.id(), 0x5058_4701 + ordinal as u32);
    }
    assert_eq!(
        provider.markers().nth(221).unwrap().function,
        Function::AgcSetUcRegIndirectPatchAddRegisters
    );
}

#[test]
fn paired_video_aliases_preserve_old_markers_and_exact_versioned_descriptors() {
    use pentax_linker::{Limits, SceLimits, SymbolBinding, SymbolType, inspect_sce};
    let provider = build_graphics_provider(PAGE_SIZE).unwrap();
    let view = pentax_loader::open_executable(provider.image()).unwrap();
    let metadata = inspect_sce(&view, &Limits::DEFAULT, &SceLimits::DEFAULT)
        .unwrap()
        .into_metadata()
        .unwrap();
    let descriptors = metadata.descriptors().decoded().unwrap();
    let module = &descriptors.module_info()[4];
    let library = &descriptors.export_libraries()[4];
    assert_eq!(module.name().as_bytes(), b"libSceVideoOut");
    assert_eq!((module.version_major(), module.version_minor()), (1, 1));
    assert_eq!(library.name().as_bytes(), b"libSceVideoOut");
    assert_eq!(library.version(), 1);
    assert_eq!(provider.function_count(), 283);
    assert_eq!(metadata.link().symbols().len(), 284);
    for (index, (old, nid, function)) in [
        (117, "zgXifHT9ErY", Function::VideoIsFlipPending),
        (108, "HXzjK9yI30k", Function::VideoAddFlipEvent),
        (222, "1FZBKy8HeNU", Function::VideoGetVblankStatus),
        (223, "DYhhWbJSeRg", Function::VideoColorSettingsSetGamma),
        (224, "pv9CI5VC+R0", Function::VideoAdjustColor),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 273 + index;
        assert_eq!(ENTRIES[old], (nid, function, 3, 3));
        assert_eq!(ENTRIES[at], (nid, function, 4, 4));
        let symbol = &metadata.link().symbols()[at + 1];
        assert_eq!(
            symbol.name.as_ref().unwrap().as_bytes(),
            format!("{nid}#F#F").as_bytes()
        );
        assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
        assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
        assert_eq!(ENTRIES.iter().filter(|row| row.0 == nid).count(), 2);
        for ordinal in [old, at] {
            let marker = provider.markers().nth(ordinal).unwrap();
            assert_eq!(marker.function, function);
            assert_eq!(marker.id(), 0x5058_4701 + ordinal as u32);
            assert_eq!(marker.rva(), 0x100 + ordinal as u64 * 16);
            assert_eq!(
                &provider.image()[marker.rva() as usize..marker.rva() as usize + 8],
                &marker.bytes()
            );
        }
        assert_eq!(symbol.value, provider.markers().nth(at).unwrap().rva());
    }
    let previous = provider.markers().nth(272).unwrap();
    assert_eq!(previous.function, Function::AgcDcbGetLodStats);
    assert_eq!((previous.rva(), previous.id()), (0x1200, 0x5058_4811));
    assert_eq!(provider.image().len(), PAGE_SIZE as usize);
    assert!(matches!(
        build_graphics_provider(PAGE_SIZE - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn submit_acb_alias_keeps_prior_markers_and_fits_the_exact_provider_bound() {
    use pentax_linker::{Limits, SceLimits, SymbolBinding, SymbolType, inspect_sce};
    let provider = build_graphics_provider(PAGE_SIZE).unwrap();
    let view = pentax_loader::open_executable(provider.image()).unwrap();
    let metadata = inspect_sce(&view, &Limits::DEFAULT, &SceLimits::DEFAULT)
        .unwrap()
        .into_metadata()
        .unwrap();
    let descriptors = metadata.descriptors().decoded().unwrap();
    let module = &descriptors.module_info()[6];
    let library = &descriptors.export_libraries()[6];
    assert_eq!(module.name().as_bytes(), b"libSceAgcDriver");
    assert_eq!((module.version_major(), module.version_minor()), (1, 1));
    assert_eq!(library.name().as_bytes(), b"libSceAgcDriver");
    assert_eq!(library.version(), 1);
    assert_eq!(provider.function_count(), 283);
    assert_eq!(metadata.link().symbols().len(), 284);
    assert_eq!(ENTRIES[2], ("gSRnr79F8tQ", Function::Acb, 0, 0));
    assert_eq!(ENTRIES[278], ("gSRnr79F8tQ", Function::Acb, 6, 6));
    assert_eq!(
        ENTRIES.iter().filter(|row| row.0 == "gSRnr79F8tQ").count(),
        2
    );
    let symbol = &metadata.link().symbols()[279];
    assert_eq!(symbol.name.as_ref().unwrap().as_bytes(), b"gSRnr79F8tQ#H#H");
    assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
    assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
    for ordinal in [2, 277, 278] {
        let marker = provider.markers().nth(ordinal).unwrap();
        assert_eq!(marker.id(), 0x5058_4701 + ordinal as u32);
        assert_eq!(marker.rva(), 0x100 + ordinal as u64 * 16);
        assert_eq!(
            &provider.image()[marker.rva() as usize..marker.rva() as usize + 8],
            &marker.bytes()
        );
    }
    assert_eq!(symbol.value, provider.markers().nth(278).unwrap().rva());
    // This actual catalogue exactly fills the admitted one-page metadata span.
    assert_eq!(
        (NAME_BYTES, DYNAMIC_AT, STRINGS_AT, SYMBOLS_AT),
        (68, 4720, 176, 9568)
    );
    assert_eq!(
        SYMBOLS_AT + metadata.link().symbols().len() * 24,
        PAGE_SIZE as usize
    );
    assert_eq!(provider.image().len(), PAGE_SIZE as usize);
    assert!(matches!(
        build_graphics_provider(PAGE_SIZE - 1),
        Err(KernelProviderError::Budget)
    ));
}
