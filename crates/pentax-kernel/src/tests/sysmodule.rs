// SPDX-License-Identifier: GPL-2.0-only
use super::*;

const UNLOADED: u64 = 0x805a_1001;

#[test]
fn sysmodule_deferred_game_load_keeps_reference_and_output_pending_until_commit() {
    let mut owner = kernel(Limits {
        max_bytes: 4,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.put(0, &[0xa5; 8]);
    let raw = CallArgs([0xabcd_0115, 1 << 32, 2 << 32, 3 << 32, ARG, 99]);
    let admitted = owner
        .admit_sysmodule_load(Service::SysmoduleLoadInternal, raw)
        .unwrap();
    assert_eq!(admitted.path(), "/app0/sce_module/libSceNpCppWebApi.prx");
    assert_eq!(admitted.raw_arguments(), raw);
    assert_eq!(admitted.module_args(), 0);
    assert_eq!(admitted.argument_pointer(), 0);
    assert_eq!(admitted.result_pointer(), None);
    assert_eq!(guest.get(ARG, 8), [0xa5; 8]);
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(
        call(&mut owner, &mut guest, Service::SysmoduleIsLoaded, &[0x115]),
        Ok(UNLOADED)
    );
    let attempts = owner.usage().attempts;
    assert_eq!(
        owner.complete_sysmodule_load(admitted.sysmodule_completion().unwrap(), &mut guest),
        Ok(0)
    );
    assert_eq!(owner.usage().attempts, attempts);
    assert_eq!(owner.usage().bytes_written, 4);
    assert_eq!(owner.usage().handles_live, 1);
    assert_eq!(guest.get(ARG, 8), [0, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5]);
    assert_eq!(
        call(&mut owner, &mut guest, Service::SysmoduleIsLoaded, &[0x115]),
        Ok(0)
    );
}

#[test]
fn sysmodule_deferred_game_load_refuses_invalid_arguments_and_preserves_failed_output_state() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(game_module_path(0x114), None);
    assert_eq!(game_module_path(0xffff), None);
    for (service, raw) in [
        (Service::SysmoduleUnload, CallArgs([0x115, 0, 0, 0, ARG, 0])),
        (Service::SysmoduleLoad, CallArgs([0x114, 0, 0, 0, 0, 0])),
        (
            Service::SysmoduleLoadInternal,
            CallArgs([0x115, 1, 0, 0, ARG, 0]),
        ),
        (
            Service::SysmoduleLoadInternal,
            CallArgs([0x115, 0, 0, 0, 0, 0]),
        ),
    ] {
        assert!(owner.admit_sysmodule_load(service, raw).is_err());
        assert_eq!(owner.usage().handles_live, 0);
    }
    let admitted = owner
        .admit_sysmodule_load(
            Service::SysmoduleLoadInternal,
            CallArgs([0x115, 0, 0, 0, ARG + PAGE, 0]),
        )
        .unwrap();
    assert!(
        owner
            .complete_sysmodule_load(admitted.sysmodule_completion().unwrap(), &mut guest)
            .is_err()
    );
    assert_eq!(owner.usage().handles_live, 0);
}

#[test]
fn sysmodule_references_follow_uint16_identity_and_owner_lifetime() {
    let mut owner = kernel(Default::default());
    let mut other = kernel(Default::default());
    let mut guest = TestGuest::new();
    for id in [0, 0xffff, 0x8000] {
        let baseline = owner.usage().handles_live;
        assert_eq!(
            call(&mut owner, &mut guest, Service::SysmoduleIsLoaded, &[id]),
            Ok(UNLOADED)
        );
        assert_eq!(
            call(&mut owner, &mut guest, Service::SysmoduleUnload, &[id]),
            Ok(UNLOADED)
        );
        assert_eq!(
            call(&mut owner, &mut guest, Service::SysmoduleLoad, &[id]),
            Ok(0)
        );
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                Service::SysmoduleLoad,
                &[id | 0xabcd_0000]
            ),
            Ok(0)
        );
        assert_eq!(owner.usage().handles_live, baseline + 1);
        assert_eq!(
            call(&mut other, &mut guest, Service::SysmoduleIsLoaded, &[id]),
            Ok(UNLOADED)
        );
        assert_eq!(
            call(&mut owner, &mut guest, Service::SysmoduleUnload, &[id]),
            Ok(0)
        );
        assert_eq!(
            call(&mut owner, &mut guest, Service::SysmoduleIsLoaded, &[id]),
            Ok(0)
        );
        assert_eq!(
            call(&mut owner, &mut guest, Service::SysmoduleUnload, &[id]),
            Ok(0)
        );
        assert_eq!(owner.usage().handles_live, baseline);
        assert_eq!(
            call(&mut owner, &mut guest, Service::SysmoduleIsLoaded, &[id]),
            Ok(UNLOADED)
        );
    }
    assert_eq!(owner.usage().bytes_read, 0);
    assert_eq!(owner.usage().bytes_written, 0);
}

#[test]
fn sysmodule_internal_output_commits_before_references_and_preserves_sentinels() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(PAGE - 8, &[0xa5; 8]);
    // Original C int arguments consume low32 only; output is exactly int32.
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SysmoduleLoadInternal,
            &[13, 1 << 32, 2 << 32, 3 << 32, ARG + PAGE - 4]
        ),
        Ok(0)
    );
    assert_eq!(
        guest.get(ARG + PAGE - 8, 8),
        [0xa5, 0xa5, 0xa5, 0xa5, 0, 0, 0, 0]
    );
    for existing in [false, true] {
        for case in 0..7 {
            let id = 40 + case;
            if existing {
                assert_eq!(
                    call(&mut owner, &mut guest, Service::SysmoduleLoad, &[id]),
                    Ok(0)
                );
            }
            let before = guest.get(ARG + PAGE - 8, 8);
            let handles = owner.usage().handles_live;
            let mut args = [id, 0, 0, 0, ARG + PAGE - 4];
            match case {
                0..=2 => args[case as usize + 1] = 1,
                3 => args[4] = 0,
                4 => args[4] += 2,
                5 => args[4] = u64::MAX - 2,
                6 => guest.refuse = true,
                _ => unreachable!(),
            }
            assert!(
                call(
                    &mut owner,
                    &mut guest,
                    Service::SysmoduleLoadInternal,
                    &args
                )
                .is_err()
            );
            guest.refuse = false;
            assert_eq!(owner.usage().handles_live, handles);
            assert_eq!(guest.get(ARG + PAGE - 8, 8), before);
            assert_eq!(
                call(&mut owner, &mut guest, Service::SysmoduleIsLoaded, &[id]),
                Ok(if existing { 0 } else { UNLOADED })
            );
            if existing {
                call(&mut owner, &mut guest, Service::SysmoduleUnload, &[id]).unwrap();
                assert_eq!(
                    call(&mut owner, &mut guest, Service::SysmoduleIsLoaded, &[id]),
                    Ok(UNLOADED)
                );
            }
        }
    }
}

#[test]
fn sysmodule_resource_limits_do_not_publish_state_or_outputs() {
    let mut guest = TestGuest::new();
    let mut owner = kernel(Limits {
        max_handles: 1,
        ..Default::default()
    });
    call(&mut owner, &mut guest, Service::SysmoduleLoad, &[9]).unwrap();
    call(&mut owner, &mut guest, Service::SysmoduleLoad, &[9]).unwrap();
    guest.put(0, &[0xa5; 8]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SysmoduleLoadInternal,
            &[10, 0, 0, 0, ARG]
        ),
        Err(Failure::Limit("handles"))
    );
    assert_eq!(guest.get(ARG, 8), [0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, Service::SysmoduleIsLoaded, &[10]),
        Ok(UNLOADED)
    );
    for _ in 0..2 {
        call(&mut owner, &mut guest, Service::SysmoduleUnload, &[9]).unwrap();
    }
    call(&mut owner, &mut guest, Service::SysmoduleLoad, &[10]).unwrap();
    assert_eq!(owner.usage().handles_live, 1);
    for limits in [
        Limits {
            max_bytes: 3,
            ..Default::default()
        },
        Limits {
            max_work: 1,
            ..Default::default()
        },
        Limits {
            max_attempts: 0,
            ..Default::default()
        },
    ] {
        let mut owner = kernel(limits);
        assert!(matches!(
            call(
                &mut owner,
                &mut guest,
                Service::SysmoduleLoadInternal,
                &[17, 0, 0, 0, ARG]
            ),
            Err(Failure::Limit(_))
        ));
        assert_eq!(owner.usage().handles_live, 0);
        assert_eq!(guest.get(ARG, 8), [0xa5; 8]);
    }
}

#[test]
fn sysmodule_registrations_keep_namespaces_and_versions_exact() {
    for (nid, service) in [
        ("g8cM39EUZ6o", Service::SysmoduleLoad),
        ("eR2bZFAAU0Q", Service::SysmoduleUnload),
        ("fMP5NHUOaMk", Service::SysmoduleIsLoaded),
        ("hHrGoGoNf+s", Service::SysmoduleLoadInternal),
    ] {
        for module in ["Sysmodule", "libSceSysmodule"] {
            for library in ["Sysmodule", "libSceSysmodule"] {
                assert_eq!(resolve(module, (1, 1), library, 1, nid), Some(service));
                assert_eq!(resolve(module, (1, 2), library, 1, nid), None);
                assert_eq!(resolve(module, (1, 1), library, 2, nid), None);
            }
        }
        assert_eq!(resolve("libkernel", (1, 1), "libkernel", 1, nid), None);
        assert_eq!(
            resolve("UnreviewedSysmodule", (1, 1), "Sysmodule", 1, nid),
            None
        );
    }
}
