use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub install: Install,
    pub registry: RegistryCfg,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct General {
    pub color: bool,
    pub confirm: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Install {
    /// Where tools are installed. Default `~/.local/bin`.
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct RegistryCfg {
    pub url: Option<String>,
    pub max_age_hours: u64,
}

impl Default for General {
    fn default() -> Self {
        General {
            color: true,
            confirm: true,
        }
    }
}
impl Default for RegistryCfg {
    fn default() -> Self {
        RegistryCfg {
            url: None,
            max_age_hours: 1,
        }
    }
}

impl Config {
    pub fn parse(text: &str, origin: &str) -> Result<Config> {
        let c: Config = toml::from_str(text).map_err(|e| {
            Error::Config(format!("Invalid configuration in {origin}:\n{e}\nFix the file, or delete it to use the defaults."))
        })?;
        if let Some(u) = &c.registry.url
            && !(u.starts_with("https://") || u.starts_with("file://"))
        {
            return Err(Error::Config(format!(
                "Invalid configuration in {origin}:\nregistry.url must start with https:// (got '{u}')."
            )));
        }
        Ok(c)
    }

    pub fn load(path: Option<&Path>) -> Result<Config> {
        let Some(path) = path else {
            return Ok(Config::default());
        };
        match std::fs::read_to_string(path) {
            Ok(t) => Config::parse(&t, &path.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(Error::Config(format!(
                "Cannot read {}: {e}",
                path.display()
            ))),
        }
    }
}

fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() && Path::new(&v).is_absolute() => Some(PathBuf::from(v)),
        _ => std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(|h| PathBuf::from(h).join(fallback)),
    }
}

#[derive(Debug, Clone)]
pub struct Dirs {
    pub config_file: Option<PathBuf>,
    pub state_file: PathBuf,
    pub registry_cache: PathBuf,
    pub default_bin: PathBuf,
}

impl Dirs {
    pub fn from_env() -> Result<Dirs> {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                Error::Config("HOME is not set; cannot decide where to install tools.".into())
            })?;
        let state =
            xdg("XDG_STATE_HOME", ".local/state").unwrap_or_else(|| home.join(".local/state"));
        let cache = xdg("XDG_CACHE_HOME", ".cache").unwrap_or_else(|| home.join(".cache"));
        Ok(Dirs {
            config_file: xdg("XDG_CONFIG_HOME", ".config")
                .map(|d| d.join("ifhx").join("config.toml")),
            state_file: state.join("ifhx").join("installed.toml"),
            registry_cache: cache.join("ifhx").join("registry.toml"),
            default_bin: home.join(".local").join("bin"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let c = Config::parse("", "t").unwrap();
        assert!(c.general.color && c.general.confirm);
        assert_eq!(c.registry.max_age_hours, 1);
        let c = Config::parse(
            "[install]\ndir='/opt/ifhx'\n[registry]\nurl='https://x/y.toml'\n",
            "t",
        )
        .unwrap();
        assert_eq!(c.install.dir, Some("/opt/ifhx".into()));
    }

    #[test]
    fn bad_values_are_actionable() {
        let e = Config::parse("[registry]\nurl='http://insecure'\n", "cfg")
            .unwrap_err()
            .to_string();
        assert!(e.contains("https://") && e.contains("cfg"), "{e}");
        assert!(Config::parse("[general]\ncolor='yes'\n", "t").is_err());
        assert_eq!(
            Config::load(Some(Path::new("/nonexistent/c.toml"))).unwrap(),
            Config::default()
        );
    }
}
