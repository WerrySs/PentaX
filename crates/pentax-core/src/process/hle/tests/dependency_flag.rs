// SPDX-License-Identifier: GPL-2.0-or-later

//! Original typed data imports, with persistent guest writes and isolated owners.
use super::super::fixture::Import;
use super::super::provider::{
    HEAP_TRACE_BYTES, HEAP_TRACE_RVA, NEED_FLAG_BYTES, NEED_FLAG_NID, NEED_FLAG_RVA,
    NEED_FLAG_SYMBOL,
};
use super::super::session::attest;
use super::*;
use crate::memory::{MemoryError, Permissions};
use crate::process::SessionLoadLimits;
use pentax_linker::{DescriptorEncoding, SymbolBinding, SymbolType, SymbolVisibility, inspect_sce};

fn consumer() -> Consumer {
    let mut consumer = minimal_consumer();
    consumer.imports = vec![Import {
        nid: *NEED_FLAG_NID,
        symbol_type: 1,
        binding: 1,
        relocation: 6,
        addend: 0,
    }];
    consumer
}

#[test]
fn dependency_flag_is_typed_writable_nx_data_preserves_writes_and_isolates_processes() {
    let provider = providers();
    let view = open_executable(provider.image()).unwrap();
    let metadata = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    // Seventeen canonical function rows precede the typed data object.
    assert_eq!(NEED_FLAG_SYMBOL, 18);
    let symbol = &metadata.metadata().unwrap().link().symbols()[NEED_FLAG_SYMBOL];
    assert_eq!(symbol.symbol_type(), SymbolType::OBJECT);
    assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
    assert_eq!(symbol.visibility(), SymbolVisibility::PROTECTED); // PentaX factory policy.
    assert_eq!(
        (symbol.value, symbol.size),
        (NEED_FLAG_RVA, NEED_FLAG_BYTES)
    );
    assert_eq!(symbol.value % 4, 0);
    assert_eq!(NEED_FLAG_RVA, HEAP_TRACE_RVA + HEAP_TRACE_BYTES);
    assert_eq!((HEAP_TRACE_RVA, NEED_FLAG_RVA), (0x4000, 0x4208));
    let image = consumer().build().unwrap();
    let plan = plan_one(&image, &provider).unwrap();
    assert_eq!(plan.provider_bindings(), 1);
    let address = PROVIDER_BIAS + NEED_FLAG_RVA;
    assert_eq!(plan.link_plan().patches()[0].value(), address);
    let mut first = prepare_hle_session(&plan, &SessionLoadLimits::default()).unwrap();
    let mut second = prepare_hle_session(&plan, &SessionLoadLimits::default()).unwrap();
    let mut value = [0; 4];
    first.session.memory.read(address, &mut value).unwrap();
    assert_eq!(u32::from_le_bytes(value), 1);
    first
        .session
        .memory
        .write(address, &37_u32.to_le_bytes())
        .unwrap();
    attest(&first.session.memory, &plan).unwrap();
    first.session.memory.read(address, &mut value).unwrap();
    assert_eq!(u32::from_le_bytes(value), 37);
    second.session.memory.read(address, &mut value).unwrap();
    assert_eq!(u32::from_le_bytes(value), 1);
    assert!(matches!(
        first.session.memory.fetch(address, &mut [0]),
        Err(MemoryError::PermissionDenied(_))
    ));
    second
        .session
        .memory
        .protect(
            PROVIDER_BIAS + HEAP_TRACE_RVA,
            crate::memory::PAGE_SIZE,
            Permissions::READ,
        )
        .unwrap();
    assert!(attest(&second.session.memory, &plan).is_err());
}

#[test]
fn dependency_flag_requires_exact_canonical_scope_version_and_object_import_type() {
    let provider = providers();
    for case in 0..8 {
        let mut original = consumer();
        match case {
            0 => original.module_name = b"libSceUnreviewedLibcInternal",
            1 => original.library_name = b"LibcInternalExt",
            2 => original.major = 2,
            3 => original.minor = 2,
            4 => original.version = 2,
            5 => original.imports[0].symbol_type = 2,
            6 => original.imports[0].relocation = 7,
            _ => original.module_name = b"unrelated-module",
        }
        assert!(plan_one(&original.build().unwrap(), &provider).is_err());
    }
}

#[test]
fn dependency_object_relocations_retain_real_address_arithmetic_across_encodings() {
    let provider = providers();
    for encoding in [
        DescriptorEncoding::Classic,
        DescriptorEncoding::Variant1Observed,
    ] {
        for dynlib in [false, true] {
            for (module, library) in [
                (b"LibcInternal".as_slice(), b"LibcInternal".as_slice()),
                (
                    b"libSceLibcInternal".as_slice(),
                    b"libSceLibcInternal".as_slice(),
                ),
                (b"LibcInternal".as_slice(), b"libSceLibcInternal".as_slice()),
                (b"libSceLibcInternal".as_slice(), b"LibcInternal".as_slice()),
            ] {
                let mut original = consumer();
                original.module_name = module;
                original.library_name = library;
                original.encoding = encoding;
                original.dynlib = dynlib;
                original.imports[0].relocation = 1;
                original.imports[0].addend = -4;
                let image = original.build().unwrap();
                let plan = plan_one(&image, &provider).unwrap();
                assert_eq!(
                    plan.link_plan().patches()[0].value(),
                    PROVIDER_BIAS + NEED_FLAG_RVA - 4
                );
                prepare_hle_session(&plan, &SessionLoadLimits::default()).unwrap();
            }
        }
    }
}
