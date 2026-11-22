//! Loads the shared config schema from disk.

use ranki_model::config::parse;
pub use ranki_model::{Config, Settings};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use crate::Cli;

/// Reads `$XDG_CONFIG_HOME/ranki/config.toml`, then the home fallback.
pub fn load() -> Config {
    let path = config_path(
        env::var_os("XDG_CONFIG_HOME").as_deref(),
        env::var_os("HOME").as_deref(),
    );
    path.map_or_else(Config::default, |config_file| read_config(&config_file))
}

/// Writes the shared config back after optimizer updates.
///
/// # Errors
/// Returns an error when the config directory or file cannot be written.
pub fn save(config: &Config) -> io::Result<()> {
    let Some(path) = config_path(
        env::var_os("XDG_CONFIG_HOME").as_deref(),
        env::var_os("HOME").as_deref(),
    ) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string(config).map_err(io::Error::other)?;
    fs::write(path, text)
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
            eprintln!("ranki: config: cannot read {}: {error}", path.display());
            Config::default()
        }
    }
}

/// CLI flags win over the config file, which wins over built-in defaults.
#[must_use]
pub fn resolve_settings(
    cli: &Cli,
    config: &Config,
    fallback_collection: Option<PathBuf>,
) -> Settings {
    let mut settings = Settings::resolve(
        cli.collection.clone(),
        cli.media_dir.clone(),
        fallback_collection,
        config,
    );
    if cli.sync {
        settings.sync.enabled = true;
    }
    if cli.no_sync {
        settings.sync.enabled = false;
    }
    if cli.sync_auto {
        settings.sync.auto = true;
    }
    if cli.no_sync_auto {
        settings.sync.auto = false;
    }
    if let Some(username) = cli.sync_username.clone() {
        settings.sync.username = Some(username);
    }
    if let Some(path) = cli.sync_hkey_file.clone() {
        settings.sync.hkey_file = Some(path);
    }
    settings
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    use clap::Parser;

    use super::{Config, config_path, resolve_settings};
    use crate::Cli;

    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap_or_else(|error| panic!("{error}"))
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
    fn configured_collection_needs_no_home_fallback() {
        let mut config = Config::default();
        config.general.collection =
            Some(String::from("/config/collection.anki2"));
        let settings = resolve_settings(&cli(&["ranki", "mcp"]), &config, None);
        assert_eq!(settings.collection, Path::new("/config/collection.anki2"));
        assert_eq!(settings.media_dir, Path::new("/config/collection.media"));
    }

    #[test]
    fn fallback_is_used_without_config_or_flags() {
        let settings = resolve_settings(
            &cli(&["ranki", "gui"]),
            &Config::default(),
            Some(PathBuf::from("/tmp/default.anki2")),
        );
        assert_eq!(settings.collection, Path::new("/tmp/default.anki2"));
        assert_eq!(settings.media_dir, Path::new("/tmp/collection.media"));
    }
    #[test]
    fn sync_cli_flags_override_config() {
        let mut config = Config::default();
        config.sync.enabled = true;
        config.sync.auto = true;
        config.sync.username = Some(String::from("config-user"));
        let settings = resolve_settings(
            &cli(&[
                "ranki",
                "--no-sync",
                "--sync-username",
                "cli-user",
                "--sync-hkey-file",
                "/keys/ranki",
                "decks",
            ]),
            &config,
            None,
        );
        assert!(!settings.sync.enabled);
        assert!(settings.sync.auto);
        assert_eq!(settings.sync.username.as_deref(), Some("cli-user"));
        assert_eq!(
            settings.sync.hkey_file.as_deref(),
            Some(Path::new("/keys/ranki"))
        );

        let settings = resolve_settings(
            &cli(&["ranki", "--sync", "--no-sync-auto", "decks"]),
            &config,
            None,
        );
        assert!(settings.sync.enabled);
        assert!(!settings.sync.auto);
    }
}
