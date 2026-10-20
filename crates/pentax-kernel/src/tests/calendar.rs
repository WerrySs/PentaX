// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn calendar_uses_configured_signed_offsets_and_exact_output_layouts() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner
        .configure_system(SystemConfig {
            timezone_offset_minutes: 330,
            daylight_saving: true,
            daylight_saving_offset_minutes: 30,
            ..Default::default()
        })
        .unwrap();
    guest.put(0, &[0xa5; 96]);
    assert_eq!(
        call(&mut owner, &mut guest, Service::Gettimezone, &[ARG]),
        Ok(0)
    );
    assert_eq!(guest.get(ARG, 4), (-330_i32).to_le_bytes());
    assert_eq!(guest.get(ARG + 4, 4), 4_i32.to_le_bytes());
    assert_eq!(guest.get(ARG + 8, 8), [0xa5; 8]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ConvertUtcToLocaltime,
            &[(-10_i64) as u64, ARG + 16, ARG + 24, ARG + 40]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(16), 21_590);
    assert_eq!(guest.u64(24), (-10_i64) as u64);
    assert_eq!(guest.get(ARG + 32, 4), 19_800_u32.to_le_bytes());
    assert_eq!(guest.get(ARG + 36, 4), 1_800_u32.to_le_bytes());
    assert_eq!(guest.u64(40), 1800);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ConvertLocaltimeToUtc,
            &[21_590, u64::MAX, ARG + 48, ARG + 56, ARG + 64]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(48), (-10_i64) as u64);
    assert_eq!(guest.get(ARG + 56, 8), guest.get(ARG, 8));
    assert_eq!(guest.get(ARG + 64, 4), 1_800_i32.to_le_bytes());
    assert_eq!(guest.get(ARG + 68, 4), [0xa5; 4]);
    owner
        .configure_system(SystemConfig {
            timezone_offset_minutes: -330,
            daylight_saving: true,
            daylight_saving_offset_minutes: -30,
            ..Default::default()
        })
        .unwrap();
    call(
        &mut owner,
        &mut guest,
        Service::ConvertUtcToLocaltime,
        &[0, ARG + 16, ARG + 24, ARG + 40],
    )
    .unwrap();
    assert_eq!(guest.u64(16), (-21_600_i64) as u64);
    assert_eq!(guest.get(ARG + 32, 4), (-19_800_i32 as u32).to_le_bytes());
    assert_eq!(guest.u64(40), (-1_800_i64) as u64);
    // A disabled seasonal adjustment contributes no seconds.
    owner
        .configure_system(SystemConfig {
            timezone_offset_minutes: -330,
            ..Default::default()
        })
        .unwrap();
    call(
        &mut owner,
        &mut guest,
        Service::ConvertUtcToLocaltime,
        &[0, ARG + 16, 0, ARG + 40],
    )
    .unwrap();
    assert_eq!(guest.u64(16), (-19_800_i64) as u64);
    assert_eq!(guest.u64(40), 0);
}

#[test]
fn calendar_errors_and_optional_outputs_preserve_all_prior_bytes() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner
        .configure_system(SystemConfig {
            timezone_offset_minutes: 60,
            ..Default::default()
        })
        .unwrap();
    guest.put(0, &[0xa5; 64]);
    let before = guest.get(ARG, 64);
    for values in [
        [0, ARG, ARG + PAGE - 8, 0],         // Late output crosses a page.
        [0, ARG, ARG + 4, 0],                // Overlapping transaction.
        [i64::MAX as u64, ARG, ARG + 16, 0], // Timestamp overflow.
    ] {
        assert!(
            call(
                &mut owner,
                &mut guest,
                Service::ConvertUtcToLocaltime,
                &values
            )
            .is_err()
        );
        assert_eq!(guest.get(ARG, 64), before);
    }
    assert_eq!(
        call(&mut owner, &mut guest, Service::Gettimezone, &[0]),
        Err(Failure::Errno(Errno::Fault))
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ConvertLocaltimeToUtc,
            &[0, 0, ARG, 0, ARG + 32]
        ),
        Err(Failure::Errno(Errno::Invalid))
    );
    let bytes = owner.usage().bytes_written;
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ConvertUtcToLocaltime,
            &[i64::MAX as u64, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(owner.usage().bytes_written, bytes);
    guest.regions[0].protection = 1;
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::ConvertUtcToLocaltime,
            &[0, ARG, ARG + 16, ARG + 32]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    ));
    assert_eq!(guest.get(ARG, 64), before);
}
