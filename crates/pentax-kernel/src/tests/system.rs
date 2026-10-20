// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use pentax_config::{DateFormat, EnterButton, TimeFormat};

#[test]
fn system_parameters_use_owned_settings_and_transactional_int32_output() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let config = SystemConfig {
        language_id: 4,
        date_format: DateFormat::MonthDayYear,
        time_format: TimeFormat::TwelveHour,
        timezone_offset_minutes: -330,
        daylight_saving: true,
        daylight_saving_offset_minutes: 60,
        parental_level: 5,
        closed_captions: true,
        screen_reader: true,
        enter_button: EnterButton::Circle,
        skip_notice_screen: false,
    };
    owner.configure_system(config).unwrap();
    for (id, value) in [
        (1, 4_i32),
        (400, 4),
        (2, 2),
        (3, 0),
        (4, -330),
        (5, 1),
        (7, 5),
        (100, 1),
        (208, 1),
        (1000, 0),
    ] {
        guest.put(0, &[0xa5; 8]);
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                Service::SystemParamGetInt,
                &[id, ARG]
            ),
            Ok(0)
        );
        assert_eq!(guest.get(ARG, 4), value.to_le_bytes());
        assert_eq!(guest.get(ARG + 4, 4), [0xa5; 4]);
    }
    assert!(
        owner
            .configure_system(SystemConfig {
                language_id: 30,
                ..config
            })
            .is_err()
    );
    call(
        &mut owner,
        &mut guest,
        Service::SystemParamGetInt,
        &[400, ARG],
    )
    .unwrap();
    assert_eq!(guest.get(ARG, 4), 4_i32.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SystemParamGetInt,
            &[999, 0]
        ),
        Ok(0x80a1_0003)
    );
    let before = guest.get(ARG, 8);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SystemParamGetInt,
            &[999, ARG]
        ),
        Err(Failure::Unsupported("system integer parameter"))
    );
    assert_eq!(guest.get(ARG, 8), before);
    guest.put(PAGE - 2, &[0x55; 2]);
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::SystemParamGetInt,
            &[1, ARG + PAGE - 2]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(ARG + PAGE - 2, 2), [0x55; 2]);
}
