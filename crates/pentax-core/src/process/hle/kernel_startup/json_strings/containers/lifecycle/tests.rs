// SPDX-License-Identifier: GPL-2.0-only
//! Original real-backing readiness, stable ownership and publication refusals.
use super::*;

fn lifecycle(
    f: &mut Fixture,
    function: JsonFunction,
    args: [u64; 3],
    maximum: u64,
) -> Result<u64, KernelStartupError> {
    f.strings.service(
        &mut f.memory.pause().unwrap(),
        &mut f.kernel,
        JsonCall {
            function,
            args: [args[0], args[1], args[2], 0, 0, 0],
            vector_args: None,
            defaults: ScalarDefaults {
                boolean: 0,
                integer: 0,
                real: 0,
            },
            empty_string: 0,
            empty_cstr: 0,
        },
        &mut f.work,
        maximum,
    )
}

#[test]
fn initializer_admits_real_defaults_once_and_void_lifecycle_preserves_live_values() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    assert!(!f.strings.containers.readiness_for_test().0);
    assert_eq!(
        lifecycle(
            &mut f,
            InitializerInitializeV1,
            [u64::MAX, 1 << 63, u64::MAX],
            128 * 1024 * 1024
        ),
        Ok(0)
    );
    assert!(f.strings.containers.readiness_for_test().0);
    let [null, array, object] = f.strings.containers.readiness_for_test().1.unwrap();
    assert_eq!(f.read::<32>(null), [0; 32]);
    assert_eq!(f.word(array), null + 48);
    assert_eq!(f.word(object), null + 56);
    assert_eq!(f.kernel.usage().handles_live, 1);
    let bytes = f.strings.guest_bytes;
    let capacity = f.strings.containers.readiness_for_test().2;
    for arguments in [[0, 0, 0], [u64::MAX, u64::MAX, u64::MAX]] {
        assert_eq!(
            lifecycle(
                &mut f,
                InitializerInitializeV1,
                arguments,
                128 * 1024 * 1024
            ),
            Ok(0)
        );
        assert_eq!(f.kernel.usage().handles_live, 1);
        assert_eq!(f.strings.guest_bytes, bytes);
        assert_eq!(f.strings.containers.readiness_for_test().2, capacity);
    }
    let root = f.base + 0x100;
    f.call(ValueTypeCtor, root, 6).unwrap();
    let wrapper = f.call(ValueGetArray, root, 0).unwrap();
    let scalar = f.base + 0x200;
    f.call(ValueIntCtor, scalar, 17).unwrap();
    f.call(ArrayPushBack, wrapper, scalar).unwrap();
    let handles = f.kernel.usage().handles_live;
    for function in [InitializerTerminate, InitializerDtor] {
        assert_eq!(
            lifecycle(&mut f, function, [u64::MAX, 0, 0], 128 * 1024 * 1024),
            Ok(0)
        );
        assert!(f.strings.containers.readiness_for_test().0);
        assert_eq!(f.call(ValueCount, root, 0), Ok(1));
        let child = f.call(ValueIndexUInt, root, 0).unwrap();
        assert_eq!(f.word(child + 16), 17);
        assert_eq!(f.kernel.usage().handles_live, handles);
    }
    assert_eq!(
        lifecycle(
            &mut f,
            InitializerSetGlobalNullAccessCallback,
            [u64::MAX, 0, u64::MAX],
            128 * 1024 * 1024
        ),
        Ok(0)
    );
    assert_eq!(
        lifecycle(
            &mut f,
            InitializerSetGlobalNullAccessCallback,
            [0, u64::MAX, 0],
            128 * 1024 * 1024
        ),
        Ok(0)
    );
    assert!(matches!(
        f.strings.null_access_registration,
        Some(NullAccessRegistration {
            callback: u64::MAX,
            context: 0
        })
    ));
    assert_eq!(f.kernel.usage().handles_live, handles);
}

#[test]
fn null_access_registration_is_owner_bound_and_preserves_receipts_on_refusal() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let mut other = Fixture::new();
    let first = NullAccessRegistration {
        callback: u64::MAX,
        context: 0x9234_5678_9abc_def0,
    };
    assert!(f.strings.null_access_registration.is_none());
    assert!(
        lifecycle(
            &mut f,
            InitializerSetGlobalNullAccessCallback,
            [u64::MAX, first.callback, first.context],
            1023
        )
        .is_err()
    );
    assert!(f.strings.null_access_registration.is_none());
    assert!(f.strings.owner.is_none());
    assert_eq!(
        lifecycle(
            &mut f,
            InitializerSetGlobalNullAccessCallback,
            [u64::MAX, first.callback, first.context],
            128 * 1024 * 1024
        ),
        Ok(0)
    );
    assert!(f.strings.null_access_registration == Some(first));
    let owner = f.strings.owner;
    assert_eq!(owner, Some(f.memory.pause().unwrap().owner_id()));
    assert_eq!(f.kernel.usage().handles_live, 0);
    assert_eq!(f.strings.guest_bytes, 0);
    let work = f.work;
    assert!(
        lifecycle(
            &mut f,
            InitializerSetGlobalNullAccessCallback,
            [0, 0, u64::MAX],
            work + 1023
        )
        .is_err()
    );
    assert_eq!(f.work, work);
    assert!(f.strings.null_access_registration == Some(first));
    assert_eq!(f.strings.owner, owner);
    // A separate actual native owner cannot overwrite or clear this receipt.
    assert_eq!(
        f.strings.service(
            &mut other.memory.pause().unwrap(),
            &mut other.kernel,
            JsonCall {
                function: InitializerSetGlobalNullAccessCallback,
                args: [u64::MAX, 0, 0, 0, 0, 0],
                vector_args: None,
                defaults: ScalarDefaults {
                    boolean: 0,
                    integer: 0,
                    real: 0
                },
                empty_string: 0,
                empty_cstr: 0
            },
            &mut f.work,
            128 * 1024 * 1024
        ),
        Err(KernelStartupError::NativeOwnerMismatch)
    );
    assert_eq!(f.work, work);
    assert!(f.strings.null_access_registration == Some(first));
    assert!(other.strings.null_access_registration.is_none());
    let replacement = NullAccessRegistration {
        callback: 1_u64 << 63,
        context: u64::MAX,
    };
    assert_eq!(
        lifecycle(
            &mut f,
            InitializerSetGlobalNullAccessCallback,
            [0, replacement.callback, replacement.context],
            128 * 1024 * 1024
        ),
        Ok(0)
    );
    assert!(f.strings.null_access_registration == Some(replacement));
    for function in [InitializerTerminate, InitializerDtor] {
        assert_eq!(
            lifecycle(&mut f, function, [u64::MAX; 3], 128 * 1024 * 1024),
            Ok(0)
        );
        assert!(f.strings.null_access_registration == Some(replacement));
    }
    // Container transactions swap their own staged registries, not this inline
    // process receipt. Default initialization likewise must preserve it.
    let array = f.base + 0x100;
    f.call(ArrayCtor, array, 0).unwrap();
    f.call(ArrayDtor, array, 0).unwrap();
    assert!(f.strings.null_access_registration == Some(replacement));
    assert_eq!(
        lifecycle(&mut f, InitializerInitializeV1, [0; 3], 128 * 1024 * 1024),
        Ok(0)
    );
    assert!(f.strings.null_access_registration == Some(replacement));
    let handles = f.kernel.usage().handles_live;
    assert_eq!(
        lifecycle(
            &mut f,
            InitializerSetGlobalNullAccessCallback,
            [u64::MAX, 0, u64::MAX],
            128 * 1024 * 1024
        ),
        Ok(0)
    );
    assert!(f.strings.null_access_registration.is_none());
    assert_eq!(f.kernel.usage().handles_live, handles);
    assert_eq!(f.strings.owner, owner);
    assert_eq!(
        lifecycle(
            &mut other,
            InitializerSetGlobalNullAccessCallback,
            [0, first.callback, first.context],
            128 * 1024 * 1024
        ),
        Ok(0)
    );
    assert!(other.strings.null_access_registration == Some(first));
    assert!(f.strings.null_access_registration.is_none());
}

#[test]
fn initializer_rechecks_live_default_bytes_permissions_and_empty_identities_without_repair() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let array = f.call(ValueGetArray, 0, 0).unwrap();
    let [null, _, _] = f.strings.containers.readiness_for_test().1.unwrap();
    let handles = f.kernel.usage().handles_live;
    f.put(null + 28, &7_u32.to_le_bytes());
    assert!(lifecycle(&mut f, InitializerInitializeV1, [0; 3], 128 * 1024 * 1024).is_err());
    assert!(!f.strings.containers.readiness_for_test().0);
    assert_eq!(f.read::<4>(null + 28), 7_u32.to_le_bytes());
    f.put(null + 28, &[0; 4]);
    assert_eq!(
        lifecycle(&mut f, InitializerInitializeV1, [0; 3], 128 * 1024 * 1024),
        Ok(0)
    );
    let page = null & !0x3fff;
    f.memory
        .protect(page, 0x4000, NativePermissions::READ)
        .unwrap();
    assert!(lifecycle(&mut f, InitializerInitializeV1, [0; 3], 128 * 1024 * 1024).is_err());
    f.memory
        .protect(page, 0x4000, NativePermissions::READ_WRITE)
        .unwrap();
    let scalar = f.base + 0x100;
    f.call(ValueIntCtor, scalar, 7).unwrap();
    f.call(ArrayPushBack, array, scalar).unwrap();
    let before = f.strings.guest_bytes;
    let retained = f.kernel.usage().handles_live;
    assert!(lifecycle(&mut f, InitializerInitializeV1, [0; 3], 128 * 1024 * 1024).is_err());
    assert_eq!(f.strings.guest_bytes, before);
    assert_eq!(f.kernel.usage().handles_live, retained);
    assert!(retained > handles);
}

#[test]
fn initializer_work_quota_and_late_mapping_failure_leave_no_ready_or_partial_defaults() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    assert!(lifecycle(&mut f, InitializerInitializeV1, [0; 3], 1023).is_err());
    assert!(!f.strings.containers.readiness_for_test().0);
    assert!(f.strings.containers.readiness_for_test().1.is_none());
    assert_eq!(f.kernel.usage().handles_live, 0);
    // The kernel's selected fresh address conflicts only at the real native
    // MapZeroed publication, after all JSON staging has succeeded.
    let target = f.base + 0x10000;
    f.memory
        .map_at(target, 0x4000, NativePermissions::READ_WRITE)
        .unwrap();
    f.put(target, &[0xa7; 64]);
    assert!(
        lifecycle(
            &mut f,
            InitializerInitializeV1,
            [u64::MAX; 3],
            128 * 1024 * 1024
        )
        .is_err()
    );
    assert_eq!(f.read::<64>(target), [0xa7; 64]);
    assert!(!f.strings.containers.readiness_for_test().0);
    assert!(f.strings.containers.readiness_for_test().1.is_none());
    assert_eq!(f.strings.guest_bytes, 0);
    assert_eq!(f.kernel.usage().handles_live, 0);
    f.memory.unmap(target, 0x4000).unwrap();
    assert_eq!(
        lifecycle(&mut f, InitializerInitializeV1, [0; 3], 128 * 1024 * 1024),
        Ok(0)
    );
    assert_eq!(f.kernel.usage().handles_live, 1);

    let mut limited = Fixture::new();
    limited.kernel = Kernel::new(
        Limits {
            virtual_base: limited.base + 0x10000,
            virtual_bytes: 0x7f0000,
            max_handles: 1,
            max_bytes: 64 * 1024 * 1024,
            max_work: 128 * 1024 * 1024,
            max_mapped_bytes: 0x7f0000,
            ..Default::default()
        },
        ProcessIdentity {
            process_id: 1,
            initial_thread_id: 1,
        },
        Vec::new(),
    )
    .unwrap();
    let string = limited.base + 0x100;
    limited.call(StringCtor, string, 0).unwrap();
    let token = limited.word(string);
    assert!(
        lifecycle(
            &mut limited,
            InitializerInitializeV1,
            [0; 3],
            128 * 1024 * 1024
        )
        .is_err()
    );
    assert!(!limited.strings.containers.readiness_for_test().0);
    assert!(limited.strings.containers.readiness_for_test().1.is_none());
    assert_eq!(limited.kernel.usage().handles_live, 1);
    assert_eq!(limited.word(string), token);
}
