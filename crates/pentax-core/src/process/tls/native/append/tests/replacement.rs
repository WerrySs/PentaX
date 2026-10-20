// SPDX-License-Identifier: GPL-2.0-only

//! Original relocation inside initialized TLS; no guest code executes.
use super::*;
use pentax_linker::{
    DT_SCE_IMPORT_LIB, DT_SCE_NEEDED_MODULE, FallbackPolicy, Resolution,
    append_symbolic_module_with_fallbacks, plan_missing_functions,
};
use pentax_platform::native_memory::{NativePermissions, PausedNativeMemory};

pub(super) fn consumer() -> Vec<u8> {
    let mut names = vec![0];
    let mut append = |value: &[u8]| {
        let offset = names.len() as u64;
        names.extend_from_slice(value);
        names.push(0);
        offset
    };
    let module = append(b"original-module");
    let library = append(b"original-library");
    let symbol = append(b"AAAAAAAAAAA#D#C");
    let mut bytes = vec![0; PAGE_SIZE as usize + 8];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (at, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 3)] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [
        (20, 1_u32),
        (64, 1),
        (68, 5),
        (120, 2),
        (124, 4),
        (176, 7),
        (180, 4),
    ] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let length = bytes.len() as u64;
    for (at, value) in [
        (24, 0x100_u64),
        (32, 64),
        (96, length),
        (104, 2 * PAGE_SIZE),
        (112, PAGE_SIZE),
        (128, 0x200),
        (136, 0x200),
        (152, 11 * 16),
        (160, 11 * 16),
        (168, 8),
        (184, PAGE_SIZE),
        (192, PAGE_SIZE),
        (208, 8),
        (216, 32),
        (224, 16),
        (0x600, PAGE_SIZE),
        (0x608, (1 << 32) | 1),
    ] {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[0x100] = 0xc3;
    bytes[0x418..0x41c].copy_from_slice(&(symbol as u32).to_le_bytes());
    bytes[0x41c] = 0x12;
    bytes[0x500..0x500 + names.len()].copy_from_slice(&names);
    for (index, (tag, value)) in [
        (5_i64, 0x500_u64),
        (10, names.len() as u64),
        (6, 0x400),
        (39, 48),
        (11, 24),
        (
            DT_SCE_NEEDED_MODULE,
            module | (3 << 32) | (2 << 40) | (2 << 48),
        ),
        (DT_SCE_IMPORT_LIB, library | (7 << 32) | (3 << 48)),
        (7, 0x600),
        (8, 24),
        (9, 24),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        bytes[0x200 + index * 16..0x208 + index * 16].copy_from_slice(&tag.to_le_bytes());
        bytes[0x208 + index * 16..0x210 + index * 16].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub(super) fn read_word(paused: &PausedNativeMemory<'_>, address: u64) -> u64 {
    let mut bytes = [0; 8];
    paused.read(address, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

#[test]
fn fallback_replacement_updates_only_attested_slots_and_future_tls_defaults_or_rolls_back() {
    for case in 0..3 {
        let bytes = consumer();
        let owner =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10_0000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), base)];
        let missing = plan_missing_functions(&inputs, &Default::default(), 1).unwrap();
        let fallback =
            crate::process::hle::build_unavailable_provider(missing, &Default::default()).unwrap();
        let mut real = fallback.image().to_vec();
        let code = fallback.exit_callback_rva() as usize + 16;
        real[code..code + 8].copy_from_slice(&[0xb8, 42, 0, 0, 0, 0xc3, 0x90, 0x90]);
        let mut supplied = inputs.to_vec();
        supplied.push(ModuleInput::new(
            open_executable(fallback.image()).unwrap(),
            base + 0x1_0000,
        ));
        let source = plan_symbolic(&supplied, &Default::default()).unwrap();
        assert_eq!(source.patches().len(), 1);
        let old = source.patches()[0];
        let Resolution::Definition(reference) = old.resolution() else {
            panic!("expected original fallback");
        };
        let bindings = plan_tls_bindings(&source, &Default::default()).unwrap();
        let policy = StaticTlsLimits {
            reserved_static_bytes: PAGE_SIZE,
            thread_control: ThreadControlBlockPolicy::KytyObservedSelfPointerV1,
            max_instances: 3,
            max_storage_bytes: 6 * PAGE_SIZE,
            ..Default::default()
        };
        let layout = plan_relocated_static_tls(&bindings, &policy).unwrap();
        let mut session = super::super::super::prepare_native_in(
            &layout,
            &StaticTlsStorage {
                thread_pointers: &[base + 0x6_8000],
            },
            &NativeLoadLimits::default(),
            Some(owner),
        )
        .unwrap();
        let live = session.instances()[0].blocks[0].address;
        session
            .pause()
            .unwrap()
            .write(live, &99_u64.to_le_bytes())
            .unwrap();
        assert_eq!(
            u64::from_le_bytes(session.initializers[0][..8].try_into().unwrap()),
            old.value()
        );
        if case == 1 {
            let page = old.address() / PAGE_SIZE * PAGE_SIZE;
            session
                .memory
                .space
                .protect(page, PAGE_SIZE, NativePermissions::READ_WRITE)
                .unwrap();
            session
                .pause()
                .unwrap()
                .write(old.address(), &0xfeed_u64.to_le_bytes())
                .unwrap();
            session
                .memory
                .space
                .protect(page, PAGE_SIZE, NativePermissions::READ_EXECUTE)
                .unwrap();
        }
        let fallback_policy = FallbackPolicy::new(
            &source,
            reference.module,
            fallback.image(),
            &[reference],
            &Default::default(),
        )
        .unwrap();
        let next = append_symbolic_module_with_fallbacks(
            &source,
            ModuleInput::new(open_executable(&real).unwrap(), base + 0x2_0000),
            &fallback_policy,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(next.fallback_transitions().len(), 1);
        let replacement = next.fallback_transitions()[0].new_patch().value();
        let next_bindings = plan_tls_bindings(&next, &Default::default()).unwrap();
        let next_layout = plan_relocated_static_tls(&next_bindings, &policy).unwrap();
        session.memory.fail_append_after_initializer = case == 2;
        let result = session.append_module(
            &next_layout,
            NativeRange {
                address: base + 0x2_0000,
                size: 0x1_0000,
            },
            &NativeLoadLimits::default(),
        );
        assert_eq!(read_word(&session.pause().unwrap(), live), 99);
        if case == 0 {
            result.unwrap();
            assert_eq!(session.memory.active_source_identity(), next.identity());
            assert_eq!(session.memory.module_count(), 3);
            assert_eq!(
                read_word(&session.pause().unwrap(), old.address()),
                replacement
            );
            assert_eq!(
                u64::from_le_bytes(session.initializers[0][..8].try_into().unwrap()),
                replacement
            );
            let added = session.append_thread_instance(base + 0x8_8000).unwrap();
            let fresh = session.instances()[added].blocks[0].address;
            assert_eq!(read_word(&session.pause().unwrap(), fresh), replacement);
            assert_eq!(read_word(&session.pause().unwrap(), live), 99);
        } else {
            assert!(result.is_err());
            assert_eq!(session.memory.active_source_identity(), source.identity());
            assert_eq!(session.memory.module_count(), 2);
            assert_eq!(
                read_word(&session.pause().unwrap(), old.address()),
                if case == 1 { 0xfeed } else { old.value() }
            );
            assert_eq!(
                u64::from_le_bytes(session.initializers[0][..8].try_into().unwrap()),
                old.value()
            );
            session
                .pause()
                .unwrap()
                .preflight_reserved_unmapped(base + 0x2_0000, PAGE_SIZE)
                .unwrap();
        }
    }
}
