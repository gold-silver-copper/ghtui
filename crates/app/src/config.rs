//! `config.toml`: theme, UI options and key bindings. Every field is
//! optional; unknown fields are rejected so typos don't pass silently.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ghtui_theme::Rgb;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub ui: UiConfig,
    /// Action name → key sequences.
    #[serde(default)]
    pub keys: HashMap<String, Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeConfig {
    #[serde(default)]
    pub mode: ModeSetting,
    /// Seed color, `#rrggbb`.
    pub seed: Option<String>,
    #[serde(default)]
    pub color_depth: DepthSetting,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ModeSetting {
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum DepthSetting {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "truecolor", alias = "24bit")]
    TrueColor,
    #[serde(rename = "256")]
    Ansi256,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    #[serde(default)]
    pub nerd_font: bool,
    #[serde(default)]
    pub density: Density,
}

/// How much room list rows take.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    /// Compact on short terminals (under 30 rows), comfortable otherwise.
    #[default]
    Auto,
    /// One row per item.
    Compact,
    /// Two rows per item, as on GitHub, with rules between.
    Comfortable,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    /// Loads `path`, or the defaults if it doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                Self::parse(&text).with_context(|| format!("invalid config {}", path.display()))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub fn seed(&self) -> Result<Rgb> {
        match &self.theme.seed {
            None => Ok(ghtui_theme::DEFAULT_SEED),
            Some(s) => Rgb::parse_hex(s)
                .with_context(|| format!("theme.seed `{s}` is not a #rrggbb color")),
        }
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    use etcetera::BaseStrategy;
    let base = etcetera::choose_base_strategy().ok()?;
    Some(base.config_dir().join("ghtui").join("config.toml"))
}

pub fn cache_dir() -> Option<PathBuf> {
    use etcetera::BaseStrategy;
    let native = etcetera::base_strategy::choose_native_strategy().ok()?;
    Some(native.cache_dir().join("ghtui"))
}

/// Where state that must outlive the cache lives (review drafts).
pub fn data_dir() -> Option<PathBuf> {
    use etcetera::BaseStrategy;
    let native = etcetera::base_strategy::choose_native_strategy().ok()?;
    Some(native.data_dir().join("ghtui"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_is_default() {
        let config = Config::parse("").unwrap();
        assert_eq!(config.theme.mode, ModeSetting::Auto);
        assert_eq!(config.theme.color_depth, DepthSetting::Auto);
        assert!(!config.ui.nerd_font);
        assert_eq!(config.seed().unwrap(), ghtui_theme::DEFAULT_SEED);
    }

    #[test]
    fn parses_full_config() {
        let config = Config::parse(
            r##"
            [theme]
            mode = "light"
            seed = "#6750a4"
            color_depth = "256"

            [ui]
            nerd_font = true
            density = "compact"

            [keys]
            down = ["j", "<C-n>"]
            "##,
        )
        .unwrap();
        assert_eq!(config.theme.mode, ModeSetting::Light);
        assert_eq!(config.theme.color_depth, DepthSetting::Ansi256);
        assert_eq!(config.seed().unwrap(), Rgb::from_u32(0x6750a4));
        assert!(config.ui.nerd_font);
        assert_eq!(config.ui.density, Density::Compact);
        assert_eq!(config.keys["down"], ["j", "<C-n>"]);
    }

    #[test]
    fn rejects_typos_and_bad_values() {
        assert!(Config::parse("[theme]\nmood = \"dark\"").is_err());
        assert!(Config::parse("[theme]\nmode = \"dim\"").is_err());
        let bad_seed = Config::parse("[theme]\nseed = \"blue\"").unwrap();
        assert!(bad_seed.seed().is_err());
    }

    #[test]
    fn missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("nope.toml")).unwrap();
        assert!(config.keys.is_empty());
    }
}
