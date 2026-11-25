use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The release tag, or "main" for a build of the default branch.
    pub version: String,
    /// "release" (prebuilt binary) or "source" (built here).
    pub method: String,
    pub path: PathBuf,
    pub sha256: String,
    pub installed_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub tools: BTreeMap<String, Record>,
}

impl State {
    /// Missing file = nothing installed. A corrupt one is an error, never silently reset.
    pub fn load(path: &Path) -> Result<State> {
        match std::fs::read_to_string(path) {
            Ok(t) => toml::from_str(&t).map_err(|e| {
                Error::Config(format!(
                    "The install record {} is damaged:\n{e}\nFix or delete it (installed binaries are not affected).",
                    path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(Error::Io(format!("Cannot read {}: {e}", path.display()))),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".installed.{}.tmp", std::process::id()));
        let text = toml::to_string_pretty(self).map_err(|e| Error::Io(e.to_string()))?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_missing_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s/installed.toml");
        assert!(State::load(&p).unwrap().tools.is_empty());
        let mut s = State::default();
        s.tools.insert(
            "ifhf".into(),
            Record {
                version: "v0.1.0".into(),
                method: "release".into(),
                path: "/x/ifhf".into(),
                sha256: "ab".into(),
                installed_at: 5,
            },
        );
        s.save(&p).unwrap();
        assert_eq!(State::load(&p).unwrap().tools, s.tools);
    }

    #[test]
    fn damaged_state_is_reported_not_reset() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("installed.toml");
        std::fs::write(&p, "tools = [[[").unwrap();
        let e = State::load(&p).unwrap_err().to_string();
        assert!(e.contains("damaged") && e.contains("installed.toml"), "{e}");
    }
}
