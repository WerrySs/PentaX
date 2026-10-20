// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn controller_entries_preserve_source_user_port_errors_and_noop_contracts() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    let profile = UserConfig {
        id: 37,
        ..Default::default()
    };
    // Init/close ignore every input and don't freeze an unobserved user profile.
    assert_eq!(
        call(&mut k, &mut g, Service::PadInit, &[u64::MAX; 6]),
        Ok(0)
    );
    assert_eq!(
        call(&mut k, &mut g, Service::PadClose, &[u64::MAX; 6]),
        Ok(0)
    );
    k.configure_user(profile).unwrap();
    for (user, port, index, valid) in [
        (37, 0, 0, true),
        (37, 2, 0, true),
        (255, 16, 0, true),
        (1, 0, 0, false),
        (37, 16, 0, false),
        (255, 0, 0, false),
        (37, 0, 1, false),
        (37, u64::MAX, 0, false),
        (
            0xffff_ffff_0000_0025,
            0xffff_ffff_0000_0002,
            0xffff_ffff_0000_0000,
            true,
        ),
    ] {
        assert_eq!(
            call(
                &mut k,
                &mut g,
                Service::PadOpen,
                &[user, port, index, u64::MAX]
            ),
            Ok(if valid { 1 } else { 0x8092_0001 })
        );
        assert_eq!(
            call(&mut k, &mut g, Service::PadGetHandle, &[user, port, index]),
            Ok(if valid { 1 } else { 0x8092_0008 })
        );
    }
    assert_eq!(g.effects, 0);
    assert_eq!(k.usage().handles_live, 0);
    k.configure_user(profile).unwrap();
    assert!(k.configure_user(UserConfig::default()).is_err());
    assert!(!k.user_service_initialized());
}

#[test]
fn controller_remote_query_writes_one_byte_and_refusals_preserve_output_and_profile() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    g.put(0, &[0xa5; 8]);
    assert_eq!(
        call(&mut k, &mut g, Service::PadIsRemote, &[2, 0]),
        Ok(0x8092_0003)
    );
    assert_eq!(
        call(&mut k, &mut g, Service::PadIsRemote, &[1, 0]),
        Ok(0x8092_0001)
    );
    assert_eq!(g.get(ARG, 8), [0xa5; 8]);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::PadIsRemote,
            &[0xffff_ffff_0000_0001, ARG + PAGE - 1]
        ),
        Ok(0)
    );
    assert_eq!(g.get(ARG + PAGE - 1, 1), [0]);
    g.refuse = true;
    assert!(call(&mut k, &mut g, Service::PadIsRemote, &[1, ARG]).is_err());
    assert_eq!(g.get(ARG, 8), [0xa5; 8]);
    let mut limited = kernel(Limits {
        max_work: 0,
        ..Default::default()
    });
    assert!(call(&mut limited, &mut g, Service::PadOpen, &[1, 0, 0, u64::MAX]).is_err());
    limited
        .configure_user(UserConfig {
            id: 37,
            ..Default::default()
        })
        .unwrap();
}
