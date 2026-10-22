// SPDX-License-Identifier: GPL-2.0-only
//! Original exact canonical configured-memory import and unchanged old sites.
use super::*;

#[test]
fn configured_memory_binds_the_tail_site_without_moving_any_prior_marker_or_object_rva() {
    let provider = factory();
    assert_eq!(
        (
            provider.registration_count(),
            MODULES.len(),
            LIBRARIES.len()
        ),
        (1381, 69, 71)
    );
    assert_eq!(provider.code_bytes().len(), 0x2c28);
    assert_eq!(
        (provider.objects()[0].symbol, provider.objects()[0].rva),
        (1382, 0x10000)
    );
    assert_eq!(
        (provider.objects()[1].symbol, provider.objects()[1].rva),
        (1383, 0x10008)
    );
    for (index, marker) in provider.markers()[..1317].iter().enumerate() {
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
    let row = Service::registrations()[1317];
    assert_eq!(
        (row.service, row.nid, row.arguments),
        (Service::ConfiguredFlexibleMemorySize, "n1-v6FgU7MQ", 1)
    );
    let bytes = consumer(b"libkernel", b"libkernel", &[row])
        .build()
        .unwrap();
    let plan = link(&bytes, &provider).unwrap();
    let imports: Vec<_> = plan
        .patches()
        .iter()
        .filter(|patch| patch.consumer().index() == 0)
        .collect();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].value(), PROVIDER_BIAS + 0x100 + 1317 * 8);
    assert!(
        matches!(imports[0].resolution(), Resolution::Definition(reference) if reference.module.index() == 1 && reference.symbol == 1318)
    );
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn configured_memory_refuses_namespace_version_type_and_neighbor_nid_substitutions() {
    let provider = factory();
    let row = Service::registrations()[1317];
    for (module, library) in [
        (b"libSceKernel".as_slice(), b"libkernel".as_slice()),
        (b"libkernel".as_slice(), b"Posix".as_slice()),
    ] {
        assert!(matches!(
            link(
                &consumer(module, library, &[row]).build().unwrap(),
                &provider
            ),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
        let mut original = consumer(b"libkernel", b"libkernel", &[row]);
        (original.major, original.minor, original.version) = (major, minor, version);
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    let mut object = consumer(b"libkernel", b"libkernel", &[row]);
    object.imports[0].symbol_type = 1;
    object.imports[0].relocation = 6;
    assert!(matches!(
        link(&object.build().unwrap(), &provider),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
    let mut neighbor = consumer(b"libkernel", b"libkernel", &[row]);
    neighbor.imports[0].nid = *b"n1-v6FgU7MA";
    assert!(pentax_linker::NidToken::parse(&neighbor.imports[0].nid).is_ok());
    assert!(matches!(
        link(&neighbor.build().unwrap(), &provider),
        Err(SymbolicError::MissingProvider { .. })
    ));
}
