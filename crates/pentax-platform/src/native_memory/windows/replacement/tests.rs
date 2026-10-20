// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn original() -> (
    NativeAddressSpace,
    u64,
    Vec<NativeWordReplacement>,
    Vec<Page>,
) {
    let mut owner =
        NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 3 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let words = vec![
        NativeWordReplacement {
            address: base + NATIVE_GUEST_PAGE - 3,
            expected: 37,
            replacement: 43,
        },
        NativeWordReplacement {
            address: base + 2 * NATIVE_GUEST_PAGE + 17,
            expected: 51,
            replacement: 61,
        },
    ];
    owner
        .pause()
        .unwrap()
        .write(base + 0x80, b"untouched")
        .unwrap();
    for word in &words {
        owner
            .pause()
            .unwrap()
            .write(word.address, &word.expected.to_le_bytes())
            .unwrap();
    }
    owner
        .protect(
            base,
            NATIVE_GUEST_PAGE,
            NativePermissions::READ_EXECUTE.with_gpu(true, false),
        )
        .unwrap();
    owner
        .protect(
            base + NATIVE_GUEST_PAGE,
            NATIVE_GUEST_PAGE,
            NativePermissions::READ.with_gpu(false, true),
        )
        .unwrap();
    owner
        .protect(
            base + 2 * NATIVE_GUEST_PAGE,
            NATIVE_GUEST_PAGE,
            NativePermissions::new(false, false, false),
        )
        .unwrap();
    let pages = owner.pages.clone();
    (owner, base, words, pages)
}

fn assert_state(
    owner: &mut NativeAddressSpace,
    base: u64,
    words: &[NativeWordReplacement],
    pages: &[Page],
    changed: bool,
) {
    for (actual, expected) in owner.pages.iter().zip(pages) {
        assert_eq!(
            (actual.address, actual.permissions, actual.lifetime),
            (expected.address, expected.permissions, expected.lifetime)
        );
        let mut query = windows_sys::Win32::System::Memory::MEMORY_BASIC_INFORMATION::default();
        // SAFETY: Numeric address is an owned live mapping; query output is the
        // complete writable Windows ABI structure and no memory is dereferenced.
        let result = unsafe {
            windows_sys::Win32::System::Memory::VirtualQuery(
                actual.address as usize as *const c_void,
                &mut query,
                std::mem::size_of_val(&query),
            )
        };
        assert_ne!(result, 0);
        assert_eq!(query.Protect, physical(expected.permissions));
    }
    // Read verification temporarily opens our own diagnostic fixture only after
    // checking exact restored rights, including the original NOACCESS page.
    owner
        .protect(base, 3 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    for word in words {
        let mut bytes = [0; 8];
        owner
            .pause()
            .unwrap()
            .read(word.address, &mut bytes)
            .unwrap();
        assert_eq!(
            u64::from_le_bytes(bytes),
            if changed {
                word.replacement
            } else {
                word.expected
            }
        );
    }
    let mut sentinel = [0; 9];
    owner
        .pause()
        .unwrap()
        .read(base + 0x80, &mut sentinel)
        .unwrap();
    assert_eq!(&sentinel, b"untouched");
}

#[test]
fn loader_words_cross_pages_preserve_current_cpu_gpu_rights_and_unrelated_bytes() {
    let (mut owner, base, words, pages) = original();
    let mapped = owner.mapped_bytes();
    owner
        .compare_replace_words(&words, &Default::default())
        .unwrap();
    assert_eq!(owner.mapped_bytes(), mapped);
    assert_state(&mut owner, base, &words, &pages, true);
}

#[test]
fn loader_words_compare_all_before_copy_and_refuse_limits_or_overlaps() {
    for case in 0..6 {
        let (mut owner, base, mut words, pages) = original();
        let mut limits = NativeTransactionLimits::default();
        let expected = match case {
            0 => {
                words[1].expected ^= 1;
                NativeMemoryError::ReplacementMismatch
            }
            1 => {
                limits.max_outputs = 1;
                NativeMemoryError::BudgetExceeded
            }
            2 => {
                limits.max_output_bytes = 15;
                NativeMemoryError::BudgetExceeded
            }
            3 => {
                limits.max_scratch_bytes = 0;
                NativeMemoryError::BudgetExceeded
            }
            4 => {
                limits.max_preflight_steps = 0;
                NativeMemoryError::BudgetExceeded
            }
            _ => {
                words[1].address = words[0].address + 1;
                NativeMemoryError::Overlap
            }
        };
        assert_eq!(owner.compare_replace_words(&words, &limits), Err(expected));
        // Restore request data to verify the actual original backing words.
        words[1].expected = 51;
        words[1].address = base + 2 * NATIVE_GUEST_PAGE + 17;
        assert_state(&mut owner, base, &words, &pages, false);
    }
}

#[test]
fn loader_words_rollback_every_forward_stage_or_poison_on_failed_rollback() {
    // Three opened pages, two copied words, three final protection restores.
    for failure in 0..8 {
        let (mut owner, base, words, pages) = original();
        owner.operations = 0;
        owner.gate = Some(failure);
        assert!(matches!(
            owner.compare_replace_words(&words, &Default::default()),
            Err(NativeMemoryError::OperatingSystem { .. })
        ));
        owner.gate = None;
        assert_state(&mut owner, base, &words, &pages, false);
    }
    let (mut owner, _, words, _) = original();
    owner.operations = 0;
    owner.gate = Some(6); // One page already sealed when final restoration fails.
    owner.rollback_operations = 0;
    owner.rollback_gate = Some(0);
    assert_eq!(
        owner.compare_replace_words(&words, &Default::default()),
        Err(NativeMemoryError::Poisoned)
    );
    assert!(matches!(owner.pause(), Err(NativeMemoryError::Poisoned)));
}

#[test]
fn loader_words_refuse_unlisted_physical_aliases_but_allow_single_shared_view() {
    let mut owner =
        NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    {
        let mut paused = owner.pause().unwrap();
        paused
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::AllocatePhysical {
                        physical: 0,
                        size: NATIVE_GUEST_PAGE,
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
        for address in [base, base + 2 * NATIVE_GUEST_PAGE] {
            paused
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::MapPhysical {
                            address,
                            size: NATIVE_GUEST_PAGE,
                            physical: 0,
                            permissions: NativePermissions::READ_WRITE,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
        }
        paused.write(base, &37_u64.to_le_bytes()).unwrap();
    }
    let words = [NativeWordReplacement {
        address: base,
        expected: 37,
        replacement: 43,
    }];
    assert_eq!(
        owner.compare_replace_words(&words, &Default::default()),
        Err(NativeMemoryError::Overlap)
    );
    let mut bytes = [0; 8];
    owner
        .pause()
        .unwrap()
        .read(base + 2 * NATIVE_GUEST_PAGE, &mut bytes)
        .unwrap();
    assert_eq!(u64::from_le_bytes(bytes), 37);
    owner
        .unmap(base + 2 * NATIVE_GUEST_PAGE, NATIVE_GUEST_PAGE)
        .unwrap();
    owner
        .compare_replace_words(&words, &Default::default())
        .unwrap();
    owner.pause().unwrap().read(base, &mut bytes).unwrap();
    assert_eq!(u64::from_le_bytes(bytes), 43);
}
