//! Shared configuration schema, validation, and resolved settings.

#![expect(
    clippy::unnecessary_wraps,
    reason = "serde field validators must return Result"
)]

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

/// Which visual theme the review window should use.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    Dark,
    Light,
    #[default]
    System,
}

/// Scheduler available for a review session.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum SchedulerKind {
    #[default]
    Fsrs,
    Sm2,
}

impl SchedulerKind {
    /// The other scheduler, used by the session choice shortcut.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Fsrs => Self::Sm2,
            Self::Sm2 => Self::Fsrs,
        }
    }

    /// Short name shown in the sidebar and review header.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fsrs => "FSRS",
            Self::Sm2 => "SM-2",
        }
    }
}

/// Settings after CLI flags, config, and defaults are merged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings mirror independently toggled CLI and sync options"
)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub collection: PathBuf,
    pub media_dir: PathBuf,
    pub theme: ThemeChoice,
    pub font_size: f32,
    pub review_batch_size: usize,
    pub show_keyboard_hints: bool,
    /// `None` follows the desktop animation preference.
    pub reduce_motion: Option<bool>,
    pub scheduler: SchedulerKind,
    pub desired_retention: f32,
    pub auto_optimize: bool,
    pub fsrs_params: Vec<f32>,
    pub fsrs_decay: f32,
    pub last_optimized: u32,
    pub reviews_since: u32,
    pub sync: SyncSettings,
}

/// Resolved sync settings. Passwords are never part of this struct.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "sync has independent enable and automatic flags"
)]
pub struct SyncSettings {
    pub enabled: bool,
    pub auto: bool,
    pub username: Option<String>,
    pub hkey_file: Option<PathBuf>,
    pub retries: u8,
    pub backoff_ms: u64,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto: true,
            username: None,
            hkey_file: None,
            retries: 2,
            backoff_ms: 1_000,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            collection: PathBuf::new(),
            media_dir: PathBuf::new(),
            theme: ThemeChoice::System,
            font_size: 16.0,
            review_batch_size: 20,
            show_keyboard_hints: true,
            reduce_motion: None,
            scheduler: SchedulerKind::Fsrs,
            desired_retention: 0.9,
            auto_optimize: true,
            fsrs_params: Vec::new(),
            fsrs_decay: 0.1542,
            last_optimized: 0,
            reviews_since: 0,
            sync: SyncSettings::default(),
        }
    }
}

/// Default media directory next to a collection file.
#[must_use]
pub fn media_dir_for(collection: &Path) -> PathBuf {
    let parent = collection.parent().unwrap_or_else(|| Path::new("."));
    parent.join("collection.media")
}

impl Settings {
    /// CLI flags win over the config file, which wins over built-in defaults.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "CLI, config, and fallback paths are separate inputs"
    )]
    pub fn resolve(
        cli_collection: Option<PathBuf>,
        cli_media_dir: Option<PathBuf>,
        fallback_collection: Option<PathBuf>,
        config: &Config,
    ) -> Self {
        let collection = cli_collection
            .or_else(|| config.general.collection.clone().map(PathBuf::from))
            .or(fallback_collection)
            .unwrap_or_default();
        let media_dir = cli_media_dir
            .or_else(|| config.general.media_dir.clone().map(PathBuf::from))
            .unwrap_or_else(|| media_dir_for(&collection));
        Self {
            collection,
            media_dir,
            theme: config.ui.theme,
            font_size: config.ui.font_size,
            review_batch_size: config.ui.review_batch_size,
            show_keyboard_hints: config.ui.show_keyboard_hints,
            reduce_motion: config.ui.reduce_motion,
            scheduler: config.scheduler.algorithm,
            desired_retention: config.scheduler.desired_retention,
            auto_optimize: config.scheduler.auto_optimize,
            fsrs_params: config.scheduler.params.clone(),
            fsrs_decay: config.scheduler.decay,
            last_optimized: config.scheduler.last_optimized,
            reviews_since: config.scheduler.reviews_since,
            sync: SyncSettings {
                enabled: config.sync.enabled,
                auto: config.sync.auto,
                username: config.sync.username.clone(),
                hkey_file: config.sync.hkey_file.as_ref().map(PathBuf::from),
                retries: config.sync.retries,
                backoff_ms: config.sync.backoff_ms,
            },
        }
    }
}

/// Raw TOML configuration. Unknown keys and out-of-range values fall back to
/// defaults with a warning naming the full key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: GeneralConfig,
    pub ui: UiConfig,
    pub scheduler: SchedulerConfig,
    pub sync: SyncConfig,
}

/// Raw sync settings. The host key is a path or environment value only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "sync has independent enable and automatic flags"
)]
#[serde(default, deny_unknown_fields)]
pub struct SyncConfig {
    #[serde(deserialize_with = "sync_enabled_field")]
    pub enabled: bool,
    #[serde(deserialize_with = "sync_auto_field")]
    pub auto: bool,
    #[serde(deserialize_with = "sync_username_field")]
    pub username: Option<String>,
    #[serde(deserialize_with = "hkey_file_field")]
    pub hkey_file: Option<String>,
    #[serde(deserialize_with = "retries_field")]
    pub retries: u8,
    #[serde(deserialize_with = "backoff_field")]
    pub backoff_ms: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto: true,
            username: None,
            hkey_file: None,
            retries: 2,
            backoff_ms: 1_000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeneralConfig {
    #[serde(deserialize_with = "collection_field")]
    pub collection: Option<String>,
    #[serde(deserialize_with = "media_dir_field")]
    pub media_dir: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    #[serde(deserialize_with = "theme_field")]
    pub theme: ThemeChoice,
    #[serde(deserialize_with = "font_size_field")]
    pub font_size: f32,
    #[serde(deserialize_with = "batch_size_field")]
    pub review_batch_size: usize,
    #[serde(deserialize_with = "hints_field")]
    pub show_keyboard_hints: bool,
    #[serde(default, deserialize_with = "reduced_motion_field")]
    pub reduce_motion: Option<bool>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            font_size: 16.0,
            review_batch_size: 20,
            show_keyboard_hints: true,
            reduce_motion: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulerConfig {
    #[serde(deserialize_with = "algorithm_field")]
    pub algorithm: SchedulerKind,
    #[serde(deserialize_with = "retention_field")]
    pub desired_retention: f32,
    #[serde(deserialize_with = "auto_optimize_field")]
    pub auto_optimize: bool,
    #[serde(deserialize_with = "params_field")]
    pub params: Vec<f32>,
    #[serde(deserialize_with = "decay_field")]
    pub decay: f32,
    #[serde(deserialize_with = "day_field")]
    pub last_optimized: u32,
    #[serde(deserialize_with = "reviews_field")]
    pub reviews_since: u32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            algorithm: SchedulerKind::Fsrs,
            desired_retention: 0.9,
            auto_optimize: true,
            params: Vec::new(),
            decay: 0.1542,
            last_optimized: 0,
            reviews_since: 0,
        }
    }
}

const GENERAL_KEYS: &[&str] = &["collection", "media_dir"];
const UI_KEYS: &[&str] = &[
    "theme",
    "font_size",
    "review_batch_size",
    "show_keyboard_hints",
    "reduce_motion",
];
const SCHEDULER_KEYS: &[&str] = &[
    "algorithm",
    "desired_retention",
    "auto_optimize",
    "params",
    "decay",
    "last_optimized",
    "reviews_since",
];
const SYNC_KEYS: &[&str] = &[
    "enabled",
    "auto",
    "username",
    "hkey_file",
    "retries",
    "backoff_ms",
];

/// Parses a config file, warning and falling back field by field.
#[must_use]
pub fn parse(input: &str) -> Config {
    let Ok(value) = toml::from_str::<toml::Value>(input) else {
        warn("config is not valid TOML; using defaults");
        return Config::default();
    };
    let Some(root) = value.as_table() else {
        warn("config root must be a table; using defaults");
        return Config::default();
    };
    Config {
        general: section(root, "general", GENERAL_KEYS),
        ui: section(root, "ui", UI_KEYS),
        scheduler: section(root, "scheduler", SCHEDULER_KEYS),
        sync: section(root, "sync", SYNC_KEYS),
    }
}

fn section<T: DeserializeOwned + Default>(
    root: &toml::Table,
    name: &str,
    allowed: &[&str],
) -> T {
    let Some(value) = root.get(name) else {
        return T::default();
    };
    let Some(table) = value.as_table() else {
        warn(&format!("[{name}] must be a table; using defaults"));
        return T::default();
    };
    let mut cleaned = table.clone();
    cleaned.retain(|key, _| {
        if allowed.contains(&key) {
            true
        } else {
            warn(&format!("{name}.{key} is unknown; ignoring"));
            false
        }
    });
    match toml::Value::Table(cleaned).try_into::<T>() {
        Ok(section) => section,
        Err(error) => {
            warn(&format!(
                "[{name}] could not be read: {error}; using defaults"
            ));
            T::default()
        }
    }
}

fn warn(message: &str) {
    eprintln!("ranki: config: {message}");
}

fn invalid(key: &str) {
    warn(&format!("{key} has an invalid value; using its default"));
}

fn choice_text<'de, D>(deserializer: D, key: &str) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    match String::deserialize(deserializer) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => {
            invalid(key);
            Ok(String::new())
        }
    }
}

fn optional_text<'de, D>(
    deserializer: D,
    key: &str,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<String>::deserialize(deserializer) {
        Ok(Some(value)) if !value.trim().is_empty() => Ok(Some(value)),
        _ => {
            invalid(key);
            Ok(None)
        }
    }
}

fn collection_field<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    optional_text(deserializer, "general.collection")
}

fn media_dir_field<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    optional_text(deserializer, "general.media_dir")
}

fn theme_field<'de, D>(deserializer: D) -> Result<ThemeChoice, D::Error>
where
    D: Deserializer<'de>,
{
    let value = choice_text(deserializer, "ui.theme")?;
    match value.as_str() {
        "dark" => Ok(ThemeChoice::Dark),
        "light" => Ok(ThemeChoice::Light),
        "system" => Ok(ThemeChoice::System),
        _ => {
            invalid("ui.theme");
            Ok(ThemeChoice::System)
        }
    }
}

fn font_size_field<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    match f32::deserialize(deserializer) {
        Ok(value) if value.is_finite() && (8.0..=40.0).contains(&value) => {
            Ok(value)
        }
        _ => {
            invalid("ui.font_size");
            Ok(16.0)
        }
    }
}

fn batch_size_field<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    match usize::deserialize(deserializer) {
        Ok(value) if (1..=500).contains(&value) => Ok(value),
        _ => {
            invalid("ui.review_batch_size");
            Ok(20)
        }
    }
}

fn hints_field<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    bool::deserialize(deserializer).map_or_else(
        |_| {
            invalid("ui.show_keyboard_hints");
            Ok(true)
        },
        Ok,
    )
}

fn reduced_motion_field<'de, D>(
    deserializer: D,
) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<bool>::deserialize(deserializer).map_or_else(
        |_| {
            invalid("ui.reduce_motion");
            Ok(None)
        },
        Ok,
    )
}

fn algorithm_field<'de, D>(deserializer: D) -> Result<SchedulerKind, D::Error>
where
    D: Deserializer<'de>,
{
    let value = choice_text(deserializer, "scheduler.algorithm")?;
    match value.as_str() {
        "fsrs" => Ok(SchedulerKind::Fsrs),
        "sm2" => Ok(SchedulerKind::Sm2),
        _ => {
            invalid("scheduler.algorithm");
            Ok(SchedulerKind::Fsrs)
        }
    }
}

fn retention_field<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    match f32::deserialize(deserializer) {
        Ok(value) if value.is_finite() && (0.7..=0.99).contains(&value) => {
            Ok(value)
        }
        _ => {
            invalid("scheduler.desired_retention");
            Ok(0.9)
        }
    }
}

fn auto_optimize_field<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    bool::deserialize(deserializer).map_or_else(
        |_| {
            invalid("scheduler.auto_optimize");
            Ok(true)
        },
        Ok,
    )
}

fn params_field<'de, D>(deserializer: D) -> Result<Vec<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    match Vec::<f32>::deserialize(deserializer) {
        Ok(value)
            if matches!(value.len(), 0 | 17 | 19 | 21)
                && value.iter().all(|weight| weight.is_finite()) =>
        {
            Ok(value)
        }
        _ => {
            invalid("scheduler.params");
            Ok(Vec::new())
        }
    }
}

fn decay_field<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    match f32::deserialize(deserializer) {
        Ok(value) if value.is_finite() && (0.1..=0.8).contains(&value) => {
            Ok(value)
        }
        _ => {
            invalid("scheduler.decay");
            Ok(0.1542)
        }
    }
}

fn day_field<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    u32::deserialize(deserializer).map_or_else(
        |_| {
            invalid("scheduler.last_optimized");
            Ok(0)
        },
        Ok,
    )
}

fn reviews_field<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    u32::deserialize(deserializer).map_or_else(
        |_| {
            invalid("scheduler.reviews_since");
            Ok(0)
        },
        Ok,
    )
}

fn sync_enabled_field<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    bool::deserialize(deserializer).map_or_else(
        |_| {
            invalid("sync.enabled");
            Ok(false)
        },
        Ok,
    )
}

fn sync_auto_field<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    bool::deserialize(deserializer).map_or_else(
        |_| {
            invalid("sync.auto");
            Ok(true)
        },
        Ok,
    )
}

fn sync_username_field<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    optional_text(deserializer, "sync.username")
}

fn hkey_file_field<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    optional_text(deserializer, "sync.hkey_file")
}

fn retries_field<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    match u8::deserialize(deserializer) {
        Ok(value) if value <= 5 => Ok(value),
        _ => {
            invalid("sync.retries");
            Ok(2)
        }
    }
}

fn backoff_field<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    match u64::deserialize(deserializer) {
        Ok(value) if value <= 10_000 => Ok(value),
        _ => {
            invalid("sync.backoff_ms");
            Ok(1_000)
        }
    }
}
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        Config, SchedulerKind, Settings, ThemeChoice, media_dir_for, parse,
    };

    #[test]
    fn defaults_are_stable() {
        assert_eq!(parse(""), Config::default());
        assert_eq!(Settings::default().theme, ThemeChoice::System);
        assert_eq!(Settings::default().scheduler, SchedulerKind::Fsrs);
    }

    #[test]
    fn full_config_is_parsed() {
        let config = parse(
            "[general]\ncollection = '/data/c.anki2'\nmedia_dir = '/data/media'\n\
             [ui]\ntheme = 'light'\nfont_size = 18.0\nreview_batch_size = 40\n\
             show_keyboard_hints = false\n\
             [scheduler]\nalgorithm = 'sm2'\ndesired_retention = 0.85\n",
        );
        assert_eq!(config.general.collection.as_deref(), Some("/data/c.anki2"));
        assert_eq!(config.ui.theme, ThemeChoice::Light);
        assert_eq!(config.ui.font_size, 18.0);
        assert_eq!(config.ui.review_batch_size, 40);
        assert!(!config.ui.show_keyboard_hints);
        assert_eq!(config.scheduler.algorithm, SchedulerKind::Sm2);
        assert_eq!(config.scheduler.desired_retention, 0.85);
    }

    #[test]
    fn invalid_values_and_unknown_keys_fall_back_by_field() {
        let config = parse(
            "[ui]\nthem = 'dark'\ntheme = 'solarized'\nfont_size = -4\n\
             review_batch_size = 0\nshow_keyboard_hints = 'yes'\n\
             [scheduler]\nalgorithm = 'magic'\ndesired_retention = 2.0\n",
        );
        assert_eq!(config, Config::default());
    }

    #[test]
    fn reduced_motion_setting_is_optional_and_validated() {
        assert_eq!(
            parse("[ui]\nreduce_motion = true\n").ui.reduce_motion,
            Some(true)
        );
        assert_eq!(
            parse("[ui]\nreduce_motion = false\n").ui.reduce_motion,
            Some(false)
        );
        assert_eq!(parse("").ui.reduce_motion, None);
        assert_eq!(
            parse("[ui]\nreduce_motion = 'yes'\n").ui.reduce_motion,
            None
        );
        let config = parse("[ui]\nreduce_motion = true\n");
        let settings = Settings::resolve(None, None, None, &config);
        assert_eq!(settings.reduce_motion, Some(true));
    }

    #[test]
    fn retention_outside_the_anki_range_falls_back() {
        let config = parse("[scheduler]\ndesired_retention = 0.6\n");
        assert_eq!(config.scheduler.desired_retention, 0.9);
        let config = parse("[scheduler]\ndesired_retention = 0.8\n");
        assert_eq!(config.scheduler.desired_retention, 0.8);
    }

    #[test]
    fn malformed_toml_uses_defaults() {
        assert_eq!(parse("[general\ncollection = "), Config::default());
    }

    #[test]
    fn resolution_prefers_cli_then_config_then_fallback() {
        let mut config = Config::default();
        config.general.collection = Some(String::from("/config/c.anki2"));
        config.general.media_dir = Some(String::from("/config/media"));
        let settings = Settings::resolve(
            Some(PathBuf::from("/cli/c.anki2")),
            Some(PathBuf::from("/cli/media")),
            Some(PathBuf::from("/default.anki2")),
            &config,
        );
        assert_eq!(settings.collection, Path::new("/cli/c.anki2"));
        assert_eq!(settings.media_dir, Path::new("/cli/media"));

        let settings = Settings::resolve(
            None,
            None,
            Some(PathBuf::from("/default")),
            &config,
        );
        assert_eq!(settings.collection, Path::new("/config/c.anki2"));
        assert_eq!(settings.media_dir, Path::new("/config/media"));

        let settings = Settings::resolve(
            None,
            None,
            Some(PathBuf::from("/tmp/default.anki2")),
            &Config::default(),
        );
        assert_eq!(settings.collection, Path::new("/tmp/default.anki2"));
        assert_eq!(settings.media_dir, Path::new("/tmp/collection.media"));
        assert_eq!(
            media_dir_for(Path::new("/data/c.anki2")),
            Path::new("/data/collection.media")
        );
    }
    #[test]
    fn sync_section_is_parsed() {
        let config = parse(
            "[sync]\nenabled = true\nauto = false\nusername = 'alice'\n\
             hkey_file = '/keys/ranki'\nretries = 4\nbackoff_ms = 250\n",
        );
        assert!(config.sync.enabled);
        assert!(!config.sync.auto);
        assert_eq!(config.sync.username.as_deref(), Some("alice"));
        assert_eq!(config.sync.hkey_file.as_deref(), Some("/keys/ranki"));
        assert_eq!(config.sync.retries, 4);
        assert_eq!(config.sync.backoff_ms, 250);
    }

    #[test]
    fn sync_defaults_and_invalid_values_are_field_local() {
        assert_eq!(parse("").sync, super::SyncConfig::default());
        let config = parse(
            "[sync]\nenabled = 'yes'\nauto = 1\nusername = ''\n\
             hkey_file = ''\nretries = 9\nbackoff_ms = 99_999\n",
        );
        assert_eq!(config.sync, super::SyncConfig::default());
    }

    #[test]
    fn scheduler_optimization_config_round_trips() {
        let config = parse(
            "[scheduler]\nauto_optimize = false\n\
             params = [0.2, 1.3, 2.3, 8.2, 6.4, 0.8, 3.0, 0.0, 1.8, 0.1, \
             0.7, 1.4, 0.0, 0.2, 1.6, 0.6, 1.8, 0.5, 0.0, 0.0, 0.15]\n\
             decay = 0.2\nlast_optimized = 19_800\nreviews_since = 42\n",
        );
        assert!(!config.scheduler.auto_optimize);
        assert_eq!(config.scheduler.params.len(), 21);
        assert_eq!(config.scheduler.decay, 0.2);
        assert_eq!(config.scheduler.last_optimized, 19_800);
        assert_eq!(config.scheduler.reviews_since, 42);
        let text = toml::to_string(&config).unwrap();
        assert_eq!(parse(&text), config);
    }

    #[test]
    fn invalid_optimization_fields_fall_back() {
        let config = parse(
            "[scheduler]\nparams = [1.0, 2.0]\ndecay = 3.0\n\
             last_optimized = -1\nreviews_since = -2\nauto_optimize = 'no'\n",
        );
        assert_eq!(config.scheduler, super::SchedulerConfig::default());
    }

    #[test]
    fn resolution_carries_sync_and_optimizer_fields() {
        let mut config = Config::default();
        config.sync.enabled = true;
        config.sync.auto = false;
        config.sync.username = Some(String::from("alice"));
        config.sync.retries = 1;
        config.scheduler.auto_optimize = false;
        config.scheduler.params = vec![0.2; 21];
        config.scheduler.decay = 0.3;
        config.scheduler.last_optimized = 12;
        config.scheduler.reviews_since = 9;
        let settings = Settings::resolve(None, None, None, &config);
        assert!(settings.sync.enabled);
        assert!(!settings.sync.auto);
        assert_eq!(settings.sync.username.as_deref(), Some("alice"));
        assert_eq!(settings.sync.retries, 1);
        assert!(!settings.auto_optimize);
        assert_eq!(settings.fsrs_params, vec![0.2; 21]);
        assert_eq!(settings.fsrs_decay, 0.3);
        assert_eq!(settings.last_optimized, 12);
        assert_eq!(settings.reviews_since, 9);
    }
}
