// SPDX-License-Identifier: GPL-2.0-only

//! Guest-visible system preferences, independent of launcher localization.

use crate::{ConfigError, invalid};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DateFormat {
    YearMonthDay,
    #[default]
    DayMonthYear,
    MonthDayYear,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeFormat {
    TwelveHour,
    #[default]
    TwentyFourHour,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnterButton {
    Circle,
    #[default]
    Cross,
}

/// Preferences reported to the guest. Reporting accessibility or parental
/// preferences does not implement a host screen reader or parental enforcement.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SystemConfig {
    /// Pinned console language ID, 0 through 29; default 1.
    pub language_id: u8,
    pub date_format: DateFormat,
    pub time_format: TimeFormat,
    /// Base UTC offset, excluding the separately reported daylight-saving flag.
    pub timezone_offset_minutes: i16,
    pub daylight_saving: bool,
    /// Active daylight-saving adjustment, used only when daylight_saving is true.
    pub daylight_saving_offset_minutes: i16,
    /// Zero disables the reported parental preference; 1 through 11 are levels.
    pub parental_level: u8,
    pub closed_captions: bool,
    pub screen_reader: bool,
    pub enter_button: EnterButton,
    /// Guest-reported notice preference; source Set/Disable calls do not alter it.
    pub skip_notice_screen: bool,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            language_id: 1,
            date_format: DateFormat::default(),
            time_format: TimeFormat::default(),
            timezone_offset_minutes: 0,
            daylight_saving: false,
            daylight_saving_offset_minutes: 60,
            parental_level: 0,
            closed_captions: false,
            screen_reader: false,
            enter_button: EnterButton::default(),
            skip_notice_screen: false,
        }
    }
}

impl SystemConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.language_id > 29 {
            return Err(invalid("system.language_id", "expected 0 through 29"));
        }
        if !(-840..=840).contains(&self.timezone_offset_minutes) {
            return Err(invalid(
                "system.timezone_offset_minutes",
                "expected -840 through 840",
            ));
        }
        if self.parental_level > 11 {
            return Err(invalid("system.parental_level", "expected 0 through 11"));
        }
        if !(-180..=180).contains(&self.daylight_saving_offset_minutes) {
            return Err(invalid(
                "system.daylight_saving_offset_minutes",
                "expected -180 through 180",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::GlobalConfig;

    #[test]
    fn notice_preference_is_additive_default_false_and_round_trips_when_enabled() {
        let old =
            GlobalConfig::from_toml("schema_version = 1\n[system]\nlanguage_id = 4\n").unwrap();
        assert!(!old.system.skip_notice_screen);
        assert_eq!(old.system.language_id, 4);
        let enabled =
            GlobalConfig::from_toml("schema_version = 1\n[system]\nskip_notice_screen = true\n")
                .unwrap();
        assert!(enabled.system.skip_notice_screen);
        assert!(enabled.effective(None).unwrap().system.skip_notice_screen);
        assert_eq!(
            GlobalConfig::from_toml(&enabled.to_toml().unwrap()).unwrap(),
            enabled
        );
    }
}
