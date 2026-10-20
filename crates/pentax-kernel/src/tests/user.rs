// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use Service::*;

fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn profile() -> UserConfig {
    UserConfig {
        id: 37,
        name: UserName::new("PentaX 🦀").unwrap(),
    }
}

#[test]
fn user_queries_share_the_configured_guest_profile_and_preserve_exact_output_spans() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.configure_user(profile()).unwrap();
    guest.put(0, &[0xa5; 80]);
    assert!(!owner.user_service_initialized());
    assert_eq!(call(&mut owner, &mut guest, UserGetInitial, &[ARG]), Ok(0));
    assert_eq!(guest.u64(0), 0xa5a5_a5a5_0000_0025);
    assert_eq!(
        call(&mut owner, &mut guest, UserGetLoginList, &[ARG]),
        Ok(0)
    );
    assert_eq!(
        guest.get(ARG, 16),
        [37_i32, -1, -1, -1]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(guest.get(ARG + 16, 8), [0xa5; 8]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            UserGetName,
            &[37, ARG + 24, u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 24, 12), "PentaX 🦀\0".as_bytes());
    assert_eq!(guest.get(ARG + 36, 4), [0xa5; 4]);
    for (service, expected) in [
        (UserGetNumber, 1_u32),
        (UserGetAgeLevel, 9),
        (UserGetVibration, 1),
        (UserGetTriggerEffect, 1),
        (UserGetChatTranscription, 0),
        (UserGetPressAndHoldDelay, 0),
        (UserGetZoomEnabled, 0),
        (UserGetZoomFollowFocus, 0),
    ] {
        guest.put(48, &[0xa5; 8]);
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                service,
                &[0xffff_ffff_0000_0025, ARG + 48]
            ),
            Ok(0)
        );
        assert_eq!(guest.u64(48), 0xa5a5_a5a5_0000_0000 | u64::from(expected));
    }
    for service in [UserInitialize, UserInitialize2] {
        assert_eq!(call(&mut owner, &mut guest, service, &[u64::MAX; 6]), Ok(0));
    }
    assert!(owner.user_service_initialized());
    assert_eq!(owner.configure_user(profile()), Ok(()));
    assert!(owner.configure_user(UserConfig::default()).is_err());
}

#[test]
fn user_login_event_is_process_owned_retryable_and_not_replayed_by_initialization() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.configure_user(profile()).unwrap();
    guest.put(0, &[0xa5; 16]);
    guest.refuse = true;
    assert!(matches!(
        call(&mut owner, &mut guest, UserGetEvent, &[ARG]),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(guest.get(ARG, 16), [0xa5; 16]);
    assert!(matches!(
        call(&mut owner, &mut guest, UserGetEvent, &[ARG + PAGE - 4]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(call(&mut owner, &mut guest, UserGetEvent, &[ARG]), Ok(0));
    assert_eq!(guest.u64(0), 37_u64 << 32);
    assert_eq!(guest.get(ARG + 8, 8), [0xa5; 8]);
    guest.put(0, &[0x5a; 8]);
    assert_eq!(call(&mut owner, &mut guest, UserInitialize, &[0]), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, UserGetEvent, &[ARG]),
        Ok(0x8096_0007)
    );
    assert_eq!(guest.get(ARG, 8), [0x5a; 8]);
    let mut other = kernel(Default::default());
    other.configure_user(profile()).unwrap();
    assert_eq!(call(&mut other, &mut guest, UserGetEvent, &[ARG]), Ok(0));
}

#[test]
fn user_errors_follow_source_order_and_game_preset_partial_clears_preserve_other_bytes() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.configure_user(profile()).unwrap();
    for service in [UserGetInitial, UserGetEvent, UserGetLoginList] {
        assert!(matches!(
            call(&mut owner, &mut guest, service, &[0]),
            Err(Failure::Unsupported(_))
        ));
    }
    assert_eq!(
        call(&mut owner, &mut guest, UserGetName, &[9, 0, 0]),
        Ok(0x8096_0009)
    );
    assert!(matches!(
        call(&mut owner, &mut guest, UserGetName, &[37, ARG, 5]),
        Err(Failure::Unsupported(_))
    ));
    for service in [
        UserGetNumber,
        UserGetGamePresets,
        UserGetAgeLevel,
        UserGetVibration,
    ] {
        assert_eq!(
            call(&mut owner, &mut guest, service, &[9, 0]),
            Ok(0x8096_0005)
        );
        assert_eq!(
            call(&mut owner, &mut guest, service, &[9, ARG]),
            Ok(0x8096_0009)
        );
    }
    for requested in [0_u64, 1, 7, 8, 9, 39, 40, 41, u64::MAX] {
        guest.put(0, &[0xa5; 48]);
        guest.put(0, &requested.to_le_bytes());
        assert_eq!(
            call(&mut owner, &mut guest, UserGetGamePresets, &[37, ARG]),
            Ok(0)
        );
        let cleared = if requested == 0 || requested > 40 {
            40
        } else {
            requested as usize
        };
        let footprint = cleared.max(8);
        assert_eq!(guest.u64(0), 40);
        assert_eq!(guest.get(ARG + 8, footprint - 8), vec![0; footprint - 8]);
        assert_eq!(
            guest.get(ARG + footprint as u64, 48 - footprint),
            vec![0xa5; 48 - footprint]
        );
    }
    guest.put(0, &9_u64.to_le_bytes());
    guest.refuse = true;
    assert!(matches!(
        call(&mut owner, &mut guest, UserGetGamePresets, &[37, ARG]),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(guest.u64(0), 9);
}

#[test]
fn user_budget_refusal_precedes_initialization_and_profile_output_publication() {
    let mut owner = kernel(Limits {
        max_work: 0,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    assert!(matches!(
        call(&mut owner, &mut guest, UserInitialize, &[0]),
        Err(Failure::Limit(_))
    ));
    assert!(!owner.user_service_initialized());
    let mut owner = kernel(Limits {
        max_scratch_bytes: 32,
        max_name_bytes: 32,
        max_io_bytes: 32,
        ..Default::default()
    });
    owner.configure_user(profile()).unwrap();
    guest.put(0, &[0xa5; 48]);
    assert_eq!(
        call(&mut owner, &mut guest, UserGetName, &[37, ARG, 64]),
        Err(Failure::Limit("scratch"))
    );
    assert_eq!(guest.get(ARG, 48), [0xa5; 48]);
    assert_eq!(owner.configure_user(UserConfig::default()), Ok(()));
    assert!(
        owner
            .configure_user(UserConfig {
                id: -1,
                ..Default::default()
            })
            .is_err()
    );
}
