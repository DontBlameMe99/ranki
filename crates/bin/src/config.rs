//! TOML configuration shared by the GUI and the command line.
//!
//! Bad values never stop startup: each field falls back independently and
//! prints a warning to stderr.

use std::{
    env,
    ffi::OsStr,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use ranki_ui::{SchedulerKind, ThemeChoice};
use serde::de::DeserializeOwned;

use crate::Cli;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub(crate) general: GeneralConfig,
    pub(crate) ui: UiConfig,
    pub(crate) scheduler: SchedulerConfig,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeneralConfig {
    pub(crate) collection: Option<String>,
    pub(crate) media_dir: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiConfig {
    pub(crate) theme: ThemeChoice,
    pub(crate) font_size: f32,
    pub(crate) review_batch_size: usize,
    pub(crate) show_keyboard_hints: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            font_size: 16.0,
            review_batch_size: 20,
            show_keyboard_hints: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SchedulerConfig {
    pub(crate) algorithm: SchedulerKind,
    pub(crate) desired_retention: f32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            algorithm: SchedulerKind::Fsrs,
            desired_retention: 0.9,
        }
    }
}

/// Fully resolved settings after CLI flags, config, and defaults are merged.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub(crate) collection: PathBuf,
    pub(crate) media_dir: PathBuf,
    pub(crate) theme: ThemeChoice,
    pub(crate) font_size: f32,
    pub(crate) review_batch_size: usize,
    pub(crate) show_keyboard_hints: bool,
    pub(crate) scheduler: SchedulerKind,
    pub(crate) desired_retention: f32,
}

/// Reads `$XDG_CONFIG_HOME/ranki/config.toml`, then the home fallback.
pub fn load() -> Config {
    let path = config_path(
        env::var_os("XDG_CONFIG_HOME").as_deref(),
        env::var_os("HOME").as_deref(),
    );
    path.map_or_else(Config::default, |config_file| read_config(&config_file))
}

pub fn config_path(
    xdg: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    xdg.filter(|value| !value.is_empty()).map_or_else(
        || {
            home.filter(|value| !value.is_empty())
                .map(|dir| PathBuf::from(dir).join(".config/ranki/config.toml"))
        },
        |dir| Some(PathBuf::from(dir).join("ranki/config.toml")),
    )
}

fn read_config(path: &Path) -> Config {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == ErrorKind::NotFound => Config::default(),
        Err(error) => {
            warn(&format!("cannot read {}: {error}", path.display()));
            Config::default()
        }
    }
}

pub fn parse(input: &str) -> Config {
    let Ok(value) = toml::from_str::<toml::Value>(input) else {
        warn("config is not valid TOML; using defaults");
        return Config::default();
    };
    let Some(table) = value.as_table() else {
        warn("config root must be a table; using defaults");
        return Config::default();
    };
    let general = section(table, "general");
    let ui = section(table, "ui");
    let scheduler = section(table, "scheduler");
    Config {
        general: GeneralConfig {
            collection: text_field(general, "general.collection"),
            media_dir: text_field(general, "general.media_dir"),
        },
        ui: UiConfig {
            theme: theme_field(ui),
            font_size: field(ui, "ui.font_size", 16.0, |value: &f32| {
                value.is_finite() && (8.0..=40.0).contains(value)
            }),
            review_batch_size: field(
                ui,
                "ui.review_batch_size",
                20,
                |value: &usize| *value >= 1 && *value <= 500,
            ),
            show_keyboard_hints: field(
                ui,
                "ui.show_keyboard_hints",
                true,
                |_: &bool| true,
            ),
        },
        scheduler: SchedulerConfig {
            algorithm: algorithm_field(scheduler),
            desired_retention: field(
                scheduler,
                "scheduler.desired_retention",
                0.9,
                |value: &f32| {
                    value.is_finite() && (0.5..=0.999).contains(value)
                },
            ),
        },
    }
}

/// CLI flags win over the config file, which wins over built-in defaults.
pub fn resolve_settings(
    cli: &Cli,
    config: &Config,
    fallback_collection: PathBuf,
) -> Settings {
    let collection = cli
        .collection
        .clone()
        .or_else(|| config.general.collection.clone().map(PathBuf::from))
        .unwrap_or(fallback_collection);
    let media_dir = cli
        .media_dir
        .clone()
        .or_else(|| config.general.media_dir.clone().map(PathBuf::from))
        .unwrap_or_else(|| crate::media_dir_for(&collection));
    Settings {
        collection,
        media_dir,
        theme: config.ui.theme,
        font_size: config.ui.font_size,
        review_batch_size: config.ui.review_batch_size,
        show_keyboard_hints: config.ui.show_keyboard_hints,
        scheduler: config.scheduler.algorithm,
        desired_retention: config.scheduler.desired_retention,
    }
}

fn section<'a>(root: &'a toml::Table, name: &str) -> Option<&'a toml::Table> {
    root.get(name).and_then(|value| {
        value.as_table().or_else(|| {
            warn(&format!("[{name}] must be a table; using defaults"));
            None
        })
    })
}

fn text_field(section: Option<&toml::Table>, key: &str) -> Option<String> {
    let value = section
        .and_then(|table| table.get(key.rsplit('.').next().unwrap_or(key)))?;
    match value.as_str() {
        Some(text) if !text.trim().is_empty() => Some(text.to_owned()),
        _ => {
            warn(&format!("{key} must be a non-empty string; ignoring"));
            None
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "a config field needs its section, key, default, and validator"
)]
fn field<T: DeserializeOwned>(
    section: Option<&toml::Table>,
    key: &str,
    default: T,
    valid: impl Fn(&T) -> bool,
) -> T {
    let Some(value) = section
        .and_then(|table| table.get(key.rsplit('.').next().unwrap_or(key)))
    else {
        return default;
    };
    match T::deserialize(value.clone()) {
        Ok(checked) if valid(&checked) => checked,
        _ => {
            warn(&format!("{key} has an invalid value; using {key} default"));
            default
        }
    }
}

fn theme_field(section: Option<&toml::Table>) -> ThemeChoice {
    let Some(text) = text_field(section, "ui.theme") else {
        return ThemeChoice::System;
    };
    match text.as_str() {
        "dark" => ThemeChoice::Dark,
        "light" => ThemeChoice::Light,
        "system" => ThemeChoice::System,
        _ => {
            warn("ui.theme must be dark, light, or system; using system");
            ThemeChoice::System
        }
    }
}

fn algorithm_field(section: Option<&toml::Table>) -> SchedulerKind {
    let Some(text) = text_field(section, "scheduler.algorithm") else {
        return SchedulerKind::Fsrs;
    };
    match text.as_str() {
        "fsrs" => SchedulerKind::Fsrs,
        "sm2" => SchedulerKind::Sm2,
        _ => {
            warn("scheduler.algorithm must be fsrs or sm2; using fsrs");
            SchedulerKind::Fsrs
        }
    }
}

fn warn(message: &str) {
    eprintln!("ranki: config: {message}");
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        path::{Path, PathBuf},
    };

    use clap::Parser;
    use ranki_ui::{SchedulerKind, ThemeChoice};

    use super::{Config, config_path, parse, resolve_settings};
    use crate::Cli;

    fn cli(args: &[&str]) -> Cli {
        match Cli::try_parse_from(args) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("{error}");
                unreachable!("test arguments must parse")
            }
        }
    }

    #[test]
    fn config_path_prefers_xdg_config_home() {
        let path = config_path(
            Some(OsStr::new("/tmp/xdg")),
            Some(OsStr::new("/home/user")),
        );
        assert_eq!(
            path.as_deref(),
            Some(Path::new("/tmp/xdg/ranki/config.toml"))
        );
    }

    #[test]
    fn config_path_falls_back_to_home() {
        let path = config_path(None, Some(OsStr::new("/home/user")));
        assert_eq!(
            path.as_deref(),
            Some(Path::new("/home/user/.config/ranki/config.toml"))
        );
        assert_eq!(config_path(None, None), None);
    }

    #[test]
    fn empty_config_uses_defaults() {
        assert_eq!(parse(""), Config::default());
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
    fn invalid_values_fall_back_to_defaults() {
        let config = parse(
            "[ui]\ntheme = 'solarized'\nfont_size = -4\nreview_batch_size = 0\n\
             show_keyboard_hints = 'yes'\n\
             [scheduler]\nalgorithm = 'magic'\ndesired_retention = 2.0\n",
        );
        assert_eq!(config, Config::default());
    }

    #[test]
    fn malformed_toml_uses_defaults() {
        assert_eq!(parse("[general\ncollection = "), Config::default());
    }

    #[test]
    fn cli_flags_override_config_paths() {
        let mut config = Config::default();
        config.general.collection =
            Some(String::from("/config/collection.anki2"));
        config.general.media_dir = Some(String::from("/config/media"));
        let cli = cli(&[
            "ranki",
            "--collection",
            "/cli/collection.anki2",
            "--media-dir",
            "/cli/media",
            "gui",
        ]);
        let settings =
            resolve_settings(&cli, &config, PathBuf::from("/default.anki2"));
        assert_eq!(settings.collection, Path::new("/cli/collection.anki2"));
        assert_eq!(settings.media_dir, Path::new("/cli/media"));
    }

    #[test]
    fn config_paths_override_defaults() {
        let mut config = Config::default();
        config.general.collection =
            Some(String::from("/config/collection.anki2"));
        let cli = cli(&["ranki", "gui"]);
        let settings =
            resolve_settings(&cli, &config, PathBuf::from("/default.anki2"));
        assert_eq!(settings.collection, Path::new("/config/collection.anki2"));
        assert_eq!(settings.media_dir, Path::new("/config/collection.media"));
    }

    #[test]
    fn defaults_are_used_without_config_or_flags() {
        let cli = cli(&["ranki", "gui"]);
        let settings = resolve_settings(
            &cli,
            &Config::default(),
            PathBuf::from("/tmp/default.anki2"),
        );
        assert_eq!(settings.collection, Path::new("/tmp/default.anki2"));
        assert_eq!(settings.media_dir, Path::new("/tmp/collection.media"));
        assert_eq!(settings.theme, ThemeChoice::System);
        assert_eq!(settings.scheduler, SchedulerKind::Fsrs);
        assert_eq!(settings.font_size, 16.0);
        assert_eq!(settings.review_batch_size, 20);
        assert!(settings.show_keyboard_hints);
    }
}
