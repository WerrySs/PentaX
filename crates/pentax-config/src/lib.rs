//! Versioned PentaX preferences with strict validation and atomic persistence.
//!
//! Settings describe user preferences. Their presence does not imply that a GPU,
//! audio, input, or PS5 execution backend has implemented the requested behavior.
//! Global preferences and per-title overrides are separate TOML documents. Neither
//! document contains game locations or other machine-specific filesystem paths.

use serde::{Deserialize, Serialize};
use std::fmt;
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
mod system;
mod user;
use std::sync::atomic::{AtomicU64, Ordering};
pub use system::{DateFormat, EnterButton, SystemConfig, TimeFormat};
pub use user::{UserConfig, UserName};

/// Current on-disk schema. Ordinary saves refuse unsupported destination versions.
pub const SCHEMA_VERSION: u32 = 1;

/// Display colors selected by the user.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Follow the desktop theme.
    System,
    /// Use a dark interface.
    #[default]
    Dark,
    /// Use a light interface.
    Light,
}

/// Preferences for the launcher interface.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Preferred color scheme.
    pub theme: Theme,
    /// Accent color in `#RRGGBB` form.
    pub accent: String,
    /// Language tag such as `en-US` or `en`; translations depend on the frontend.
    pub language: String,
    /// User scale multiplier, in the inclusive range 0.75 to 2.5.
    pub scale: f32,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            accent: "#8B5CF6".into(),
            language: "en-US".into(),
            scale: 1.0,
        }
    }
}

/// Requested output resolution, bounded from 1920×1080 through 7680×4320.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    /// Horizontal pixel count.
    pub width: u32,
    /// Vertical pixel count.
    pub height: u32,
}

impl Default for Resolution {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
        }
    }
}

/// Rendering preferences for a future graphics backend.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphicsConfig {
    /// Requested output resolution. No frame generation setting is provided.
    pub resolution: Resolution,
    /// Request synchronization to the display refresh.
    pub vsync: bool,
}

impl Default for GraphicsConfig {
    fn default() -> Self {
        Self {
            resolution: Resolution::default(),
            vsync: true,
        }
    }
}

/// Audio preferences for a future audio backend.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    /// Linear master volume from zero through one.
    pub master_volume: f32,
    /// Silence output without discarding the stored volume.
    pub muted: bool,
    /// Requested buffering latency in milliseconds, from 5 through 250.
    pub latency_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            master_volume: 0.8,
            muted: false,
            latency_ms: 40,
        }
    }
}

/// Input preferences shared by the launcher and a future guest input backend.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct InputConfig {
    /// Accept connected controllers when supported by the frontend.
    pub controller_enabled: bool,
    /// Radial deadzone from zero through 0.5 for the left stick.
    pub left_stick_deadzone: f32,
    /// Radial deadzone from zero through 0.5 for the right stick.
    pub right_stick_deadzone: f32,
    /// Trigger deadzone from zero through 0.5.
    pub trigger_deadzone: f32,
    /// Mouse sensitivity multiplier from 0.1 through 10.
    pub mouse_sensitivity: f32,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            controller_enabled: true,
            left_stick_deadzone: 0.12,
            right_stick_deadzone: 0.12,
            trigger_deadzone: 0.05,
            mouse_sensitivity: 1.0,
        }
    }
}

/// Preferences for a future guest execution backend.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Requested worker count: zero means automatic; explicit counts stop at 256.
    pub cpu_threads: u16,
    /// Request additional runtime diagnostic logging.
    pub enable_debug_logging: bool,
}

/// The global preference document. A schema version is required when parsing TOML.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    /// On-disk schema version; must equal [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Launcher appearance and localization.
    #[serde(default)]
    pub ui: UiConfig,
    /// Requested rendering preferences.
    #[serde(default)]
    pub graphics: GraphicsConfig,
    /// Requested audio preferences.
    #[serde(default)]
    pub audio: AudioConfig,
    /// Controller and mouse preferences.
    #[serde(default)]
    pub input: InputConfig,
    /// Requested guest runtime preferences.
    #[serde(default)]
    pub runtime: RuntimeConfig,
    /// Guest-visible system preferences, separate from the launcher UI.
    #[serde(default)]
    pub system: SystemConfig,
    /// Local guest identity, independent of host accounts.
    #[serde(default)]
    pub user: UserConfig,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ui: UiConfig::default(),
            graphics: GraphicsConfig::default(),
            audio: AudioConfig::default(),
            input: InputConfig::default(),
            runtime: RuntimeConfig::default(),
            system: SystemConfig::default(),
            user: UserConfig::default(),
        }
    }
}

/// Optional rendering preferences for one game; omitted values inherit globally.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphicsOverrides {
    /// Override the complete output resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
    /// Override refresh synchronization, including an explicit `false` value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vsync: Option<bool>,
}

/// Optional runtime preferences for one game; omitted values inherit globally.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeOverrides {
    /// Override the requested worker count, including zero for automatic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_threads: Option<u16>,
    /// Override the additional diagnostic logging preference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_debug_logging: Option<bool>,
}

/// A per-title preference document, stored separately from global preferences.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameOverrides {
    /// On-disk schema version; must equal [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Optional rendering preferences.
    #[serde(default)]
    pub graphics: GraphicsOverrides,
    /// Optional guest runtime preferences.
    #[serde(default)]
    pub runtime: RuntimeOverrides,
}

impl Default for GameOverrides {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            graphics: GraphicsOverrides::default(),
            runtime: RuntimeOverrides::default(),
        }
    }
}

/// Validated preferences after global settings and per-title overrides are merged.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveConfig {
    /// Global launcher preferences.
    pub ui: UiConfig,
    /// Rendering preferences after inheritance.
    pub graphics: GraphicsConfig,
    /// Global audio preferences.
    pub audio: AudioConfig,
    /// Global input preferences.
    pub input: InputConfig,
    /// Guest runtime preferences after inheritance.
    pub runtime: RuntimeConfig,
    pub system: SystemConfig,
    pub user: UserConfig,
}

impl GlobalConfig {
    /// Return fresh factory defaults without reading or changing any files.
    #[must_use]
    pub fn factory_defaults() -> Self {
        Self::default()
    }

    /// Parse strict TOML and validate every setting and schema version.
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(source).map_err(ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate and encode a global preference document as TOML.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        self.validate()?;
        toml::to_string_pretty(self).map_err(ConfigError::Serialize)
    }

    /// Reject unsupported schema versions, non-finite numbers, and invalid ranges.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_schema(self.schema_version)?;
        self.system.validate()?;
        self.user.validate()?;
        let accent = self.ui.accent.as_bytes();
        if accent.len() != 7
            || accent.first() != Some(&b'#')
            || !accent[1..].iter().all(u8::is_ascii_hexdigit)
        {
            return Err(invalid("ui.accent", "expected a color in #RRGGBB form"));
        }
        if !valid_language_tag(&self.ui.language) {
            return Err(invalid(
                "ui.language",
                "expected a language tag such as en-US",
            ));
        }
        validate_float("ui.scale", self.ui.scale, 0.75, 2.5)?;
        validate_resolution(self.graphics.resolution)?;
        validate_float("audio.master_volume", self.audio.master_volume, 0.0, 1.0)?;
        if !(5..=250).contains(&self.audio.latency_ms) {
            return Err(invalid(
                "audio.latency_ms",
                "expected 5 through 250 milliseconds",
            ));
        }
        validate_float(
            "input.left_stick_deadzone",
            self.input.left_stick_deadzone,
            0.0,
            0.5,
        )?;
        validate_float(
            "input.right_stick_deadzone",
            self.input.right_stick_deadzone,
            0.0,
            0.5,
        )?;
        validate_float(
            "input.trigger_deadzone",
            self.input.trigger_deadzone,
            0.0,
            0.5,
        )?;
        validate_float(
            "input.mouse_sensitivity",
            self.input.mouse_sensitivity,
            0.1,
            10.0,
        )?;
        validate_threads(self.runtime.cpu_threads)
    }

    /// Resolve optional per-title settings without mutating either source document.
    pub fn effective(
        &self,
        overrides: Option<&GameOverrides>,
    ) -> Result<EffectiveConfig, ConfigError> {
        self.validate()?;
        let mut effective = EffectiveConfig {
            system: self.system,
            user: self.user,
            ui: self.ui.clone(),
            graphics: self.graphics.clone(),
            audio: self.audio.clone(),
            input: self.input.clone(),
            runtime: self.runtime.clone(),
        };
        if let Some(overrides) = overrides {
            overrides.validate()?;
            if let Some(value) = overrides.graphics.resolution {
                effective.graphics.resolution = value;
            }
            if let Some(value) = overrides.graphics.vsync {
                effective.graphics.vsync = value;
            }
            if let Some(value) = overrides.runtime.cpu_threads {
                effective.runtime.cpu_threads = value;
            }
            if let Some(value) = overrides.runtime.enable_debug_logging {
                effective.runtime.enable_debug_logging = value;
            }
        }
        Ok(effective)
    }
}

impl GameOverrides {
    /// Parse a strict per-title TOML document and validate its schema and values.
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let overrides: Self = toml::from_str(source).map_err(ConfigError::Parse)?;
        overrides.validate()?;
        Ok(overrides)
    }

    /// Validate and encode a per-title preference document as TOML.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        self.validate()?;
        toml::to_string_pretty(self).map_err(ConfigError::Serialize)
    }

    /// Reject unsupported schema versions and invalid override values.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_schema(self.schema_version)?;
        if let Some(value) = self.graphics.resolution {
            validate_resolution(value)?;
        }
        if let Some(value) = self.runtime.cpu_threads {
            validate_threads(value)?;
        }
        Ok(())
    }
}

/// A PS5 title identifier: `PPSA` followed by exactly five ASCII digits.
///
/// The validated representation is safe to use as a profile filename. Filesystem
/// separators, dot segments, drive prefixes, and arbitrary names are rejected.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TitleId(String);

impl TitleId {
    /// Validate and construct a PS5 title identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, ConfigError> {
        let value = value.into();
        if value.len() == 9
            && value.starts_with("PPSA")
            && value.as_bytes()[4..].iter().all(u8::is_ascii_digit)
        {
            Ok(Self(value))
        } else {
            Err(ConfigError::InvalidTitleId(value))
        }
    }

    /// Borrow the validated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TitleId {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for TitleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// File-backed configuration under a directory selected by the caller.
///
/// Global preferences use `global.toml`; profiles use `games/PPSA12345.toml`.
/// Loads never create files. Missing global preferences use factory defaults;
/// malformed, unreadable, or unsupported files return errors and remain intact.
/// Saves revalidate an existing document immediately before replacement. This is
/// not a cross-process compare-and-swap: concurrent editors are last-writer-wins,
/// and a change after the final validation can still be replaced.
#[derive(Clone, Debug)]
pub struct ConfigStore {
    directory: PathBuf,
}

impl ConfigStore {
    /// Select a configuration directory without reading or creating it.
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        let directory = directory.into();
        Self {
            directory: if directory.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                directory
            },
        }
    }

    /// The caller-selected configuration directory.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Path of the global preference document.
    #[must_use]
    pub fn global_path(&self) -> PathBuf {
        self.directory.join("global.toml")
    }

    /// Path of the preference document for a validated PS5 title identifier.
    #[must_use]
    pub fn game_path(&self, title: &TitleId) -> PathBuf {
        self.directory.join("games").join(format!("{title}.toml"))
    }

    /// Read and validate global preferences, using defaults only for a missing file.
    pub fn load_global(&self) -> Result<GlobalConfig, ConfigError> {
        match read_optional(&self.global_path())? {
            Some(source) => GlobalConfig::from_toml(&source),
            None => Ok(GlobalConfig::default()),
        }
    }

    /// Validate and atomically replace global preferences.
    ///
    /// A complete temporary file is flushed beside the destination before rename.
    /// A validation, serialization, or pre-rename I/O failure leaves an existing
    /// configuration untouched. Existing malformed or unsupported documents are
    /// refused even when the supplied configuration is valid. Replacement requires
    /// filesystem rename support. [`ConfigError::CommittedButNotDurable`] means
    /// replacement succeeded but the later directory flush failed; the new document
    /// is already visible. Unix file permissions are preserved; new files are private.
    pub fn save_global(&self, config: &GlobalConfig) -> Result<(), ConfigError> {
        atomic_write(
            &self.global_path(),
            config.to_toml()?.as_bytes(),
            |existing| GlobalConfig::from_toml(existing).map(|_| ()),
        )
    }

    /// Read and validate a title profile, returning `None` only when it is absent.
    pub fn load_game(&self, title: &TitleId) -> Result<Option<GameOverrides>, ConfigError> {
        read_optional(&self.game_path(title))?
            .map(|source| GameOverrides::from_toml(&source))
            .transpose()
    }

    /// Validate and atomically replace a title profile.
    ///
    /// Existing malformed or unsupported profiles are refused. Persistence and
    /// post-commit error semantics match [`Self::save_global`].
    pub fn save_game(&self, title: &TitleId, overrides: &GameOverrides) -> Result<(), ConfigError> {
        atomic_write(
            &self.game_path(title),
            overrides.to_toml()?.as_bytes(),
            |existing| GameOverrides::from_toml(existing).map(|_| ()),
        )
    }

    /// Load global preferences and the selected profile, then resolve inheritance.
    pub fn effective_for_game(&self, title: &TitleId) -> Result<EffectiveConfig, ConfigError> {
        let global = self.load_global()?;
        let overrides = self.load_game(title)?;
        global.effective(overrides.as_ref())
    }
}

/// Parsing, validation, or persistence failure for a preference document.
#[derive(Debug)]
pub enum ConfigError {
    /// TOML syntax, field name, or field type is invalid.
    Parse(toml::de::Error),
    /// A validated document could not be encoded as TOML.
    Serialize(toml::ser::Error),
    /// The document belongs to a different schema version.
    UnsupportedSchema {
        /// Version found in the document.
        found: u32,
        /// Version accepted by this crate.
        expected: u32,
    },
    /// A preference violates its documented constraints.
    InvalidValue {
        /// Dot-separated preference name.
        field: &'static str,
        /// Description of the accepted values.
        message: String,
    },
    /// A title identifier does not match `PPSA` and five ASCII digits.
    InvalidTitleId(String),
    /// A filesystem operation failed.
    Io {
        /// Operation that failed.
        operation: &'static str,
        /// File or directory affected by the operation.
        path: PathBuf,
        /// Underlying filesystem error.
        source: io::Error,
    },
    /// Replacement succeeded, but flushing the parent directory failed.
    ///
    /// The new file is already visible; its persistence across a sudden power loss
    /// could not be confirmed. This error does not mean the old document remains.
    CommittedButNotDurable {
        /// Configuration document that was already replaced.
        path: PathBuf,
        /// Underlying parent-directory flush error.
        source: io::Error,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(source) => write!(formatter, "invalid configuration TOML: {source}"),
            Self::Serialize(source) => write!(formatter, "cannot encode configuration: {source}"),
            Self::UnsupportedSchema { found, expected } => {
                write!(
                    formatter,
                    "unsupported configuration schema {found}; expected {expected}"
                )
            }
            Self::InvalidValue { field, message } => {
                write!(formatter, "invalid {field}: {message}")
            }
            Self::InvalidTitleId(value) => write!(
                formatter,
                "invalid PS5 title identifier {value:?}; expected PPSA and five digits"
            ),
            Self::Io {
                operation,
                path,
                source,
            } => {
                write!(formatter, "cannot {operation} {}: {source}", path.display())
            }
            Self::CommittedButNotDurable { path, source } => write!(
                formatter,
                "configuration {} was replaced, but directory durability could not be confirmed: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(source) => Some(source),
            Self::Serialize(source) => Some(source),
            Self::Io { source, .. } | Self::CommittedButNotDurable { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn validate_schema(version: u32) -> Result<(), ConfigError> {
    if version != SCHEMA_VERSION {
        return Err(ConfigError::UnsupportedSchema {
            found: version,
            expected: SCHEMA_VERSION,
        });
    }
    Ok(())
}

fn validate_resolution(resolution: Resolution) -> Result<(), ConfigError> {
    if !(1920..=7680).contains(&resolution.width) || !(1080..=4320).contains(&resolution.height) {
        return Err(invalid(
            "graphics.resolution",
            "expected dimensions from 1920x1080 through 7680x4320",
        ));
    }
    Ok(())
}

fn validate_threads(threads: u16) -> Result<(), ConfigError> {
    if threads > 256 {
        return Err(invalid(
            "runtime.cpu_threads",
            "expected zero for automatic or 1 through 256",
        ));
    }
    Ok(())
}

fn validate_float(
    field: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<(), ConfigError> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(invalid(
            field,
            format!("expected a finite number from {minimum} through {maximum}"),
        ));
    }
    Ok(())
}

fn valid_language_tag(value: &str) -> bool {
    if value.len() > 35 {
        return false;
    }
    let mut parts = value.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    (2..=3).contains(&language.len())
        && language.bytes().all(|byte| byte.is_ascii_alphabetic())
        && parts.all(|part| {
            (2..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

fn invalid(field: &'static str, message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        field,
        message: message.into(),
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> ConfigError {
    ConfigError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

fn read_optional(path: &Path) -> Result<Option<String>, ConfigError> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error("read", path, source)),
    }
}

static NEXT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

fn atomic_write(
    path: &Path,
    contents: &[u8],
    validate_existing: impl FnOnce(&str) -> Result<(), ConfigError>,
) -> Result<(), ConfigError> {
    atomic_write_with_directory_sync(path, contents, validate_existing, sync_directory)
}

fn atomic_write_with_directory_sync(
    path: &Path,
    contents: &[u8],
    validate_existing: impl FnOnce(&str) -> Result<(), ConfigError>,
    flush_directory: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), ConfigError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| io_error("create directory", parent, source))?;

    // A same-directory temporary file keeps rename within a single filesystem.
    // create_new avoids overwriting leftovers from an interrupted process.
    let (temporary_path, mut temporary_file) = loop {
        let id = NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(".pentax-{}-{id}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary_path) {
            Ok(file) => break (temporary_path, file),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(io_error(
                    "create temporary configuration",
                    &temporary_path,
                    source,
                ));
            }
        }
    };
    let result = (|| {
        temporary_file
            .write_all(contents)
            .map_err(|source| io_error("write temporary configuration", &temporary_path, source))?;
        #[cfg(unix)]
        match fs::metadata(path) {
            Ok(metadata) => temporary_file
                .set_permissions(metadata.permissions())
                .map_err(|source| {
                    io_error(
                        "preserve configuration permissions",
                        &temporary_path,
                        source,
                    )
                })?,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error("read configuration permissions", path, source)),
        }
        temporary_file
            .sync_all()
            .map_err(|source| io_error("flush temporary configuration", &temporary_path, source))?;
        drop(temporary_file);
        if let Some(existing) = read_optional(path)? {
            validate_existing(&existing)?;
        }
        fs::rename(&temporary_path, path)
            .map_err(|source| io_error("replace configuration", path, source))?;
        flush_directory(parent).map_err(|source| ConfigError::CommittedButNotDurable {
            path: path.to_owned(),
            source,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path).and_then(|directory| directory.sync_all())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    // Windows does not expose directory fsync through the standard library.
    Ok(())
}

#[cfg(test)]
mod persistence_fault_tests {
    use super::*;

    #[test]
    fn failed_directory_flush_reports_that_replacement_already_committed() {
        let directory = std::env::temp_dir().join(format!(
            "pentax-config-flush-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("global.toml");
        fs::write(&path, "old document").unwrap();
        let error = atomic_write_with_directory_sync(
            &path,
            b"new document",
            |_| Ok(()),
            |_| Err(io::Error::other("injected directory flush failure")),
        )
        .unwrap_err();
        assert!(matches!(error, ConfigError::CommittedButNotDurable { .. }));
        assert!(error.to_string().contains("was replaced"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "new document");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
