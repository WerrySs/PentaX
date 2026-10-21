// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use crate::NidToken;

fn provider() -> Module {
    let mut module = Module::provider();
    module.primary = true;
    module.symbols[0].name = b"QDLarCRo4Bc#R#f".to_vec();
    module
}

#[test]
fn qualified_lookup_selects_exact_offered_scope_before_alias_ambiguity_and_charges_refusals() {
    let mut module = provider();
    module.declarations.push(Declaration {
        role: DescriptorRole::ExportLibrary,
        encoding: DescriptorEncoding::Classic,
        name: b"other-library".to_vec(),
        id: 18,
        version: 4,
    });
    let mut alias = module.symbols[0].clone();
    alias.name = b"QDLarCRo4Bc#S#f".to_vec();
    alias.value += 16;
    module.symbols.push(alias);
    let bytes = module.build();
    let source = single(&bytes);
    assert!(matches!(
        lookup_module_export(&source, 0, b"QDLarCRo4Bc", &Default::default()).result,
        Err(ModuleLookupError::Ambiguous { .. })
    ));
    let mut query = ModuleExportQuery {
        nid: NidToken::parse(b"QDLarCRo4Bc").unwrap(),
        module: b"fixture-module",
        library: b"fixture-library",
        versions: DescriptorVersions {
            module_major: 2,
            module_minor: 3,
            library: 4,
        },
    };
    let attempt = lookup_qualified_module_export(&source, 0, &query, &Default::default());
    assert!(attempt.work > 0);
    let export = attempt.result.unwrap().unwrap();
    assert_eq!(export.address(), A_BIAS + DEFINITION);
    let refused = export.qualified_identity(0);
    assert_eq!(refused.work, 0);
    assert!(matches!(refused.result, Err(ModuleLookupError::Budget)));
    let identity = export.qualified_identity(u64::MAX);
    assert!(identity.work > 0);
    let identity = identity.result.unwrap();
    assert_eq!(
        (identity.module, identity.library, identity.versions),
        (query.module, query.library, query.versions)
    );
    assert_eq!(identity.nid, query.nid);
    assert!(identity.global_function);
    query.library = b"other-library";
    assert_eq!(
        lookup_qualified_module_export(&source, 0, &query, &Default::default())
            .result
            .unwrap()
            .unwrap()
            .address(),
        A_BIAS + DEFINITION + 16
    );
    query.versions.library = 5;
    assert!(
        lookup_qualified_module_export(&source, 0, &query, &Default::default())
            .result
            .unwrap()
            .is_none()
    );
    let limits = ModuleLookupLimits {
        max_work: 1,
        ..Default::default()
    };
    assert!(
        lookup_qualified_module_export(&source, 0, &query, &limits)
            .result
            .is_err()
    );
}
fn single(bytes: &[u8]) -> SymbolicLinkPlan<'_> {
    plan_symbolic(
        &[ModuleInput::new(open_executable(bytes).unwrap(), A_BIAS)],
        &Default::default(),
    )
    .unwrap()
}

#[test]
fn name_nid_matches_public_registration_and_independent_hashlib_padding_vectors() {
    for (name, expected) in [
        (b"sceKernelGetProcParam".as_slice(), b"959qrazPIrg"),
        (b"pentax_original_return42", b"QDLarCRo4Bc"),
        (b"", b"B4c6vzgC+Xc"),
    ] {
        assert_eq!(super::super::name_nid::for_name(name).as_bytes(), expected);
    }
    for (length, expected) in [
        (39, b"gkBYkMPPqxU"),
        (40, b"91KrSneKcJI"),
        (103, b"4Ga4sdIeePc"),
        (104, b"H3g9qoSiz3k"),
        (1024, b"p6ulHvRQrmI"),
    ] {
        assert_eq!(
            super::super::name_nid::for_name(&vec![b'x'; length]).as_bytes(),
            expected
        );
    }
}

#[test]
fn module_lookup_hash_and_raw_fallback_retain_exact_source_across_descriptor_encodings() {
    for encoding in [
        DescriptorEncoding::Classic,
        DescriptorEncoding::Variant1Observed,
    ] {
        let mut module = provider();
        module.module_encoding = encoding;
        module.library_encoding = encoding;
        let bytes = module.build();
        let source = single(&bytes);
        for name in [b"pentax_original_return42".as_slice(), b"QDLarCRo4Bc"] {
            let attempt = lookup_module_export(&source, 0, name, &Default::default());
            assert!(attempt.work > 0);
            let selected = attempt.result.unwrap().unwrap();
            assert!(std::ptr::eq(selected.source_plan(), &source));
            assert_eq!(selected.definition().module.index(), 0);
            assert_eq!(selected.address(), A_BIAS + DEFINITION);
            assert_eq!(selected.symbol_type(), crate::SymbolType::FUNC);
            assert!(!format!("{selected:?}").contains("QDLar"));
        }
    }
}

#[test]
fn module_lookup_never_falls_into_another_loaded_image_or_import_only_declarations() {
    let first = provider().build();
    let mut second = provider();
    second.primary = false;
    second.symbols[0].value += 16;
    let second = second.build();
    let source = plan(&first, &second).unwrap();
    for (module, address) in [(0, A_BIAS + DEFINITION), (1, B_BIAS + DEFINITION + 16)] {
        assert_eq!(
            lookup_module_export(
                &source,
                module,
                b"pentax_original_return42",
                &Default::default()
            )
            .result
            .unwrap()
            .unwrap()
            .address(),
            address
        );
    }
    let mut module = provider();
    module.provider = false;
    let bytes = module.build();
    let source = single(&bytes);
    assert!(
        lookup_module_export(&source, 0, b"pentax_original_return42", &Default::default())
            .result
            .unwrap()
            .is_none()
    );
}

#[test]
fn module_lookup_function_priority_precedes_object_ambiguity_and_selected_rank_ambiguity_refuses() {
    let mut module = provider();
    let mut object = module.symbols[0].clone();
    object.kind = 1;
    let mut other = object.clone();
    other.value += 8;
    module.symbols[0].value += 16;
    module.symbols.insert(0, object);
    module.symbols.insert(1, other);
    let bytes = module.build();
    let source = single(&bytes);
    assert_eq!(
        lookup_module_export(&source, 0, b"pentax_original_return42", &Default::default())
            .result
            .unwrap()
            .unwrap()
            .address(),
        A_BIAS + DEFINITION + 16
    );
    module.symbols.pop();
    let bytes = module.build();
    let source = single(&bytes);
    assert!(matches!(
        lookup_module_export(&source, 0, b"pentax_original_return42", &Default::default()).result,
        Err(ModuleLookupError::Ambiguous { .. })
    ));
    module.symbols.pop();
    let bytes = module.build();
    let source = single(&bytes);
    let selected =
        lookup_module_export(&source, 0, b"pentax_original_return42", &Default::default())
            .result
            .unwrap()
            .unwrap();
    assert_eq!(selected.symbol_type(), crate::SymbolType::OBJECT);
    assert_eq!(selected.address(), A_BIAS + DEFINITION);
}

#[test]
fn module_lookup_weak_absolute_and_unimplemented_forms_are_not_fake_missing_exports() {
    for (kind, binding, section) in [(2, 2, 1), (1, 1, 0xfff1), (0, 1, 1)] {
        let mut module = provider();
        module.symbols[0].kind = kind;
        module.symbols[0].binding = binding;
        module.symbols[0].section = section;
        let bytes = module.build();
        let source = single(&bytes);
        assert!(
            lookup_module_export(&source, 0, b"pentax_original_return42", &Default::default())
                .result
                .is_err()
        );
    }
    let bytes = provider().build();
    let source = single(&bytes);
    assert!(matches!(
        lookup_module_export(&source, 0, b"name[scope]", &Default::default()).result,
        Err(ModuleLookupError::FormattedNameUnsupported)
    ));
    assert!(matches!(
        lookup_module_export(&source, 0, b"name\0tail", &Default::default()).result,
        Err(ModuleLookupError::InvalidName)
    ));
    assert!(matches!(
        lookup_module_export(&source, 1, b"name", &Default::default()).result,
        Err(ModuleLookupError::InvalidModule)
    ));
}

#[test]
fn module_lookup_work_is_inclusive_and_retained_after_refusals() {
    let bytes = provider().build();
    let source = single(&bytes);
    let attempt =
        lookup_module_export(&source, 0, b"pentax_original_return42", &Default::default());
    let exact = ModuleLookupLimits {
        max_work: attempt.work,
        ..Default::default()
    };
    assert!(
        lookup_module_export(&source, 0, b"pentax_original_return42", &exact)
            .result
            .unwrap()
            .is_some()
    );
    let short = ModuleLookupLimits {
        max_work: exact.max_work - 1,
        ..exact
    };
    let failed = lookup_module_export(&source, 0, b"pentax_original_return42", &short);
    assert!(failed.result.is_err() && failed.work <= short.max_work && failed.work > 0);
    let short = ModuleLookupLimits {
        max_name_bytes: 0,
        ..exact
    };
    let failed = lookup_module_export(&source, 0, b"name", &short);
    assert!(failed.result.is_err() && failed.work == 0);
}
