//! The list of tools ifhx can install. Layers, most to least preferred: a fresh cache, the
//! network, a stale cache, the copy compiled into this binary. So it always works offline.

use crate::error::{Error, Result};
use crate::fetch::Fetcher;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const BUILTIN: &str = include_str!("../registry.toml");
pub const DEFAULT_URL: &str = "https://raw.githubusercontent.com/Disc0wd/ifhx/main/registry.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    pub name: String,
    /// `owner/name` on GitHub.
    pub repo: String,
    pub description: String,
    pub bin: String,
    asset: String,
}

impl Tool {
    /// A tool installed with `ifhx add`; it is never part of any registry.
    pub fn added(name: &str, repo: &str) -> Tool {
        Tool {
            name: name.into(),
            repo: repo.into(),
            description: "Added from GitHub; not an IFHX tool".into(),
            bin: name.into(),
            asset: String::new(),
        }
    }

    /// Release asset name for a platform such as `x86_64-linux`.
    pub fn asset_name(&self, platform: &str) -> String {
        self.asset.replace("{platform}", platform)
    }
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    tools: BTreeMap<String, Raw>,
}

#[derive(Deserialize)]
struct Raw {
    repo: String,
    description: String,
}

/// One line of plain text: registry strings are shown on the user's terminal, so they must not
/// carry escape sequences that could redraw the screen or fake a prompt.
fn plain(s: &str) -> bool {
    s.chars().count() <= 200 && !s.chars().any(char::is_control)
}

#[derive(Debug, Clone, Default)]
pub struct Registry(BTreeMap<String, Tool>);

pub(crate) fn ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub(crate) fn repo_ok(s: &str) -> bool {
    let mut p = s.split('/');
    let part = |x: &str| {
        !x.is_empty()
            && x.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            && x != ".."
    };
    matches!((p.next(), p.next(), p.next()), (Some(a), Some(b), None) if part(a) && part(b))
}

impl Registry {
    pub fn parse(text: &str) -> Result<Registry> {
        let file: File = toml::from_str(text)
            .map_err(|e| Error::Config(format!("The tool registry is not valid TOML:\n{e}")))?;
        let mut tools = BTreeMap::new();
        for (name, r) in file.tools {
            // The binary and asset names are always the tool's name: a registry entry cannot make a
            // signed binary land under some other name (such as `sudo`).
            let bin = name.clone();
            let asset = format!("{name}-{{platform}}");
            if !ident(&name) {
                return Err(Error::Config(format!(
                    "Registry: '{name}' is not a valid tool name."
                )));
            }
            if !repo_ok(&r.repo) {
                return Err(Error::Config(format!(
                    "Registry: tool '{name}' has an invalid repo '{}' (expected owner/name).",
                    r.repo
                )));
            }
            if !plain(&r.description) {
                return Err(Error::Config(format!(
                    "Registry: tool '{name}' has text that is not plain (control characters or too long)."
                )));
            }
            tools.insert(
                name.clone(),
                Tool {
                    name,
                    repo: r.repo,
                    description: r.description,
                    bin,
                    asset,
                },
            );
        }
        Ok(Registry(tools))
    }

    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.0.get(name)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Tool> {
        self.0.values()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub struct Loaded {
    pub registry: Registry,
    /// Warnings for the user (stale copy, built-in fallback).
    pub notes: Vec<String>,
}

fn age_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()
        .map(|d| d.as_secs())
}

fn write_atomic(path: &Path, text: &str) {
    let Some(dir) = path.parent() else { return };
    let tmp = dir.join(format!(".registry.{}.tmp", std::process::id()));
    if std::fs::create_dir_all(dir).is_ok()
        && std::fs::write(&tmp, text).is_ok()
        && std::fs::rename(&tmp, path).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
    }
}

pub fn load(
    fetcher: &dyn Fetcher,
    url: &str,
    cache: &Path,
    max_age_hours: u64,
    force: bool,
) -> Loaded {
    let cached = std::fs::read_to_string(cache)
        .ok()
        .and_then(|t| Registry::parse(&t).ok());
    if !force
        && let Some(reg) = &cached
        && age_secs(cache).is_some_and(|a| a < max_age_hours * 3600)
    {
        return Loaded {
            registry: reg.clone(),
            notes: vec![],
        };
    }
    let fetched = fetcher
        .text(url)
        .and_then(|t| Registry::parse(&t).map(|r| (t, r)));
    match fetched {
        Ok((text, registry)) => {
            write_atomic(cache, &text);
            Loaded {
                registry,
                notes: vec![],
            }
        }
        Err(e) => {
            let why = e.to_string().lines().next().unwrap_or("").to_string();
            if let Some(reg) = cached {
                let hours = age_secs(cache).unwrap_or(0) / 3600;
                Loaded {
                    registry: reg,
                    notes: vec![format!(
                        "Could not refresh the tool list ({why}); using a copy from {hours} h ago."
                    )],
                }
            } else {
                Loaded {
                    registry: Registry::parse(BUILTIN).expect("the built-in registry is valid"),
                    notes: vec![format!(
                        "Could not fetch the tool list ({why}); using the list built into this version."
                    )],
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_is_valid_and_complete() {
        let r = Registry::parse(BUILTIN).unwrap();
        let ifhf = r.get("ifhf").unwrap();
        assert_eq!(ifhf.repo, "Disc0wd/ifhf");
        assert_eq!(ifhf.bin, "ifhf");
        assert_eq!(ifhf.asset_name("x86_64-linux"), "ifhf-x86_64-linux");
        assert!(r.get("ifhx").is_some());
        assert!(r.get("ifhai").is_some());
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn overrides_are_ignored() {
        // Older registries could name a binary, an asset or a build command. None of them is honoured.
        let r = Registry::parse(
            "[tools.foo]\nrepo='a/b'\ndescription='d'\nbin='sudo'\nbuild='make'\nasset='x'\n",
        )
        .unwrap();
        let t = r.get("foo").unwrap();
        assert_eq!(t.bin, "foo");
        assert_eq!(t.asset_name("aarch64-linux"), "foo-aarch64-linux");
    }

    #[test]
    fn hostile_entries_are_rejected() {
        for bad in [
            "[tools.'../x']\nrepo='a/b'\ndescription='d'\n",
            "[tools.x]\nrepo='a'\ndescription='d'\n",
            "[tools.x]\nrepo='a/b/c'\ndescription='d'\n",
            "[tools.x]\nrepo='../b'\ndescription='d'\n",
            "[tools.x]\nrepo='a b/c'\ndescription='d'\n",
            r#"[tools.x]
repo = "a/b"
description = "fake \u001b[2J prompt""#,
            r#"[tools.x]
repo = "a/b"
description = "two\nlines""#,
            "[tools.x]\ndescription='no repo'\n",
            "not toml [",
        ] {
            assert!(Registry::parse(bad).is_err(), "{bad}");
        }
    }
}
