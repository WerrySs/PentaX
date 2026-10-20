// SPDX-License-Identifier: GPL-2.0-only

//! Bounded guest identity, independent of host accounts and online services.

use crate::{ConfigError, invalid};
use serde::{Deserialize, Serialize};

/// Validated UTF-8 display name with at most 63 bytes, excluding its terminator.
#[derive(Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct UserName {
    bytes: [u8; 63],
    length: u8,
}
impl UserName {
    pub fn new(value: &str) -> Result<Self, ConfigError> {
        if value.is_empty() || value.len() > 63 || value.chars().any(char::is_control) {
            return Err(invalid(
                "user.name",
                "expected 1 through 63 UTF-8 bytes without control characters",
            ));
        }
        let mut bytes = [0; 63];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            length: value.len() as u8,
        })
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.length)])
            .expect("validated guest user name")
    }
}
impl std::fmt::Debug for UserName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserName").finish_non_exhaustive()
    }
}
impl Default for UserName {
    fn default() -> Self {
        Self::new("PentaX").expect("fixed default user name")
    }
}
impl TryFrom<String> for UserName {
    type Error = ConfigError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}
impl From<UserName> for String {
    fn from(value: UserName) -> Self {
        value.as_str().to_owned()
    }
}

/// A single local guest session, not a host identity or an online entitlement.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserConfig {
    pub id: i32,
    pub name: UserName,
}
impl Default for UserConfig {
    fn default() -> Self {
        Self {
            id: 1,
            name: UserName::default(),
        }
    }
}
impl UserConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.id < 0 {
            return Err(invalid(
                "user.id",
                "expected a nonnegative 32-bit signed guest ID",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GlobalConfig;

    #[test]
    fn guest_user_profile_round_trips_utf8_and_survives_effective_config_resolution() {
        let config =
            GlobalConfig::from_toml("schema_version = 1\n[user]\nid = 37\nname = 'PentaX 🦀'\n")
                .unwrap();
        assert_eq!(config.user.id, 37);
        assert_eq!(config.user.name.as_str(), "PentaX 🦀");
        assert_eq!(
            GlobalConfig::from_toml(&config.to_toml().unwrap()).unwrap(),
            config
        );
        assert_eq!(config.effective(None).unwrap().user, config.user);
        assert_eq!(
            GlobalConfig::from_toml("schema_version = 1").unwrap().user,
            UserConfig::default()
        );
        let boundary = "🦀".repeat(15) + "abc";
        assert_eq!(boundary.len(), 63);
        assert_eq!(UserName::new(&boundary).unwrap().as_str(), boundary);
        assert!(!format!("{:?}", config.user.name).contains("PentaX"));
    }

    #[test]
    fn guest_user_profile_rejects_invalid_ids_names_and_unknown_fields() {
        for name in [
            "".to_owned(),
            "a".repeat(64),
            "line\nbreak".to_owned(),
            "zero\0byte".to_owned(),
        ] {
            assert!(UserName::new(&name).is_err());
        }
        for input in [
            "schema_version=1\n[user]\nid=-1",
            "schema_version=1\n[user]\nname=''",
            "schema_version=1\n[user]\nunknown=true",
        ] {
            assert!(GlobalConfig::from_toml(input).is_err());
        }
    }
}
