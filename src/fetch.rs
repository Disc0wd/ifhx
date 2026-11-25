//! Network access. Everything goes through `curl` (present on practically every Linux system),
//! so ifhx needs no TLS stack of its own and inherits the system's certificates and proxy settings.

use crate::error::{Error, Result};
use std::path::Path;
use std::process::{Command, Stdio};

pub trait Fetcher {
    /// GET a small text resource. 404 is `Error::NotFound`; anything else that fails is `Network`.
    fn text(&self, url: &str) -> Result<String>;
    /// Download to `dest`. Same error mapping; a partial file is removed on failure.
    fn file(&self, url: &str, dest: &Path) -> Result<()>;
    /// Newest release tag of a GitHub repo, or None if it has no releases.
    fn latest_tag(&self, repo: &str) -> Result<Option<String>>;
}

pub struct Curl;

const BASE: &[&str] = &[
    "--silent",
    "--show-error",
    "--location",
    "--proto",
    "=https",
    "--proto-redir",
    "=https",
    "--tlsv1.2",
    "--max-time",
    "300",
];

/// A tag we are willing to put into a URL and a file name.
pub fn valid_tag(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 64
        && !t.starts_with(['-', '.'])
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
}

impl Curl {
    fn run(&self, extra: &[&str], url: &str) -> Result<(Vec<u8>, String)> {
        let out = Command::new("curl")
            .args(BASE)
            .args(["--user-agent", concat!("ifhx/", env!("CARGO_PKG_VERSION"))])
            .args(extra)
            .arg(url)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::Network(
                        "curl was not found. Install curl (ifhx uses it for downloads).".into(),
                    )
                } else {
                    Error::Network(format!("could not run curl: {e}"))
                }
            })?;
        if !out.status.success() {
            let host = url.split('/').nth(2).unwrap_or(url);
            return Err(Error::Network(format!(
                "Could not reach {host}.\n{}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok((
            out.stdout,
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }

    fn status(code: &str, url: &str) -> Result<()> {
        match code.trim() {
            "200" => Ok(()),
            "404" => Err(Error::NotFound(format!("Not found: {url}"))),
            c => Err(Error::Network(format!(
                "The server answered HTTP {c} for {url}"
            ))),
        }
    }
}

impl Fetcher for Curl {
    fn text(&self, url: &str) -> Result<String> {
        let (out, _) = self.run(&["--write-out", "\n%{http_code}"], url)?;
        let s = String::from_utf8_lossy(&out).into_owned();
        let (body, code) = s.rsplit_once('\n').unwrap_or(("", &s));
        Self::status(code, url)?;
        Ok(body.to_string())
    }

    fn file(&self, url: &str, dest: &Path) -> Result<()> {
        let d = dest.to_string_lossy();
        let result = self
            .run(&["--write-out", "%{http_code}", "--output", &d], url)
            .and_then(|(out, _)| Self::status(&String::from_utf8_lossy(&out), url));
        if result.is_err() {
            let _ = std::fs::remove_file(dest);
        }
        result
    }

    fn latest_tag(&self, repo: &str) -> Result<Option<String>> {
        let url = format!("https://github.com/{repo}/releases/latest");
        let (out, _) = self.run(
            &[
                "--output",
                "/dev/null",
                "--write-out",
                "%{http_code} %{url_effective}",
            ],
            &url,
        )?;
        let s = String::from_utf8_lossy(&out).into_owned();
        let (code, effective) = s.split_once(' ').unwrap_or((&s, ""));
        if code == "404" {
            return Ok(None);
        }
        Ok(effective
            .split("/releases/tag/")
            .nth(1)
            .map(|t| t.trim().to_string())
            .filter(|t| valid_tag(t)))
    }
}

#[cfg(test)]
mod tests {
    use super::valid_tag;

    #[test]
    fn tags_that_are_safe_to_use_in_urls() {
        for ok in ["v0.1.0", "1.2.3", "v1.0.0-rc.1", "nightly_2026"] {
            assert!(valid_tag(ok), "{ok}");
        }
        for bad in [
            "",
            "-v1",
            ".hidden",
            "v1/../x",
            "v1 0",
            "v1;rm",
            "a".repeat(65).as_str(),
            "v1\n",
        ] {
            assert!(!valid_tag(bad), "{bad:?}");
        }
    }
}
