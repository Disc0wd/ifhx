//! Network access. Everything goes through `curl` (present on practically every Linux system),
//! so ifhx needs no TLS stack of its own and inherits the system's certificates and proxy settings.

use crate::error::{Error, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Registry, checksum and version responses are tiny; a hostile server cannot make us buffer more.
const TEXT_MAX: u64 = 1 << 20;
/// Largest download ifhx will accept. A server that sends more is stopped, and the partial file removed.
const FILE_MAX: u64 = 512 << 20;

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

/// What came back from a supervised curl run.
struct Ran {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    ok: bool,
    too_big: bool,
}

fn read_capped(mut r: impl Read, cap: u64, over: &AtomicBool) -> Vec<u8> {
    let (mut buf, mut chunk) = (Vec::new(), [0u8; 8192]);
    loop {
        match r.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) if (buf.len() + n) as u64 > cap => {
                over.store(true, Ordering::Relaxed);
                break;
            }
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    buf
}

/// Wait for `child`, killing it as soon as it prints more than `text_cap` bytes or, when it is
/// writing to `file`, that file grows past `file_cap`. curl's own size option only works when the
/// server announces a length, so this does not rely on it.
fn supervise(mut child: Child, text_cap: u64, file: Option<&Path>, file_cap: u64) -> Ran {
    let (out, err) = (child.stdout.take(), child.stderr.take());
    let over = AtomicBool::new(false);
    std::thread::scope(|s| {
        let ho = s.spawn(|| {
            out.map(|o| read_capped(o, text_cap, &over))
                .unwrap_or_default()
        });
        let he = s.spawn(|| {
            let mut v = Vec::new();
            if let Some(mut e) = err {
                let _ = (&mut e).take(64 * 1024).read_to_end(&mut v);
                let _ = std::io::copy(&mut e, &mut std::io::sink());
            }
            v
        });
        let grown = || file.is_some_and(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > file_cap));
        let (mut ok, mut too_big) = (false, false);
        loop {
            match child.try_wait() {
                Ok(Some(st)) => {
                    ok = st.success();
                    break;
                }
                Ok(None) => {}
                Err(_) => break,
            }
            if over.load(Ordering::Relaxed) || grown() {
                let _ = child.kill();
                let _ = child.wait();
                too_big = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let stdout = ho.join().unwrap_or_default();
        Ran {
            too_big: too_big || over.load(Ordering::Relaxed) || grown(),
            ok,
            stdout,
            stderr: he.join().unwrap_or_default(),
        }
    })
}

impl Curl {
    fn run(&self, extra: &[&str], url: &str, file: Option<&Path>) -> Result<(Vec<u8>, String)> {
        let child = Command::new("curl")
            .args(BASE)
            .args(["--user-agent", concat!("ifhx/", env!("CARGO_PKG_VERSION"))])
            .args(extra)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::Network(
                        "curl was not found. Install curl (ifhx uses it for downloads).".into(),
                    )
                } else {
                    Error::Network(format!("could not run curl: {e}"))
                }
            })?;
        let host = url.split('/').nth(2).unwrap_or(url);
        // A status code or a tiny body comes back on stdout; a download goes to the file.
        let cap = if file.is_some() { 64 * 1024 } else { TEXT_MAX };
        let ran = supervise(child, cap, file, FILE_MAX);
        if ran.too_big {
            return Err(Error::Network(format!(
                "The response from {host} is larger than ifhx accepts, so it was stopped."
            )));
        }
        if !ran.ok {
            return Err(Error::Network(format!(
                "Could not reach {host}.\n{}",
                String::from_utf8_lossy(&ran.stderr).trim()
            )));
        }
        Ok((
            ran.stdout,
            String::from_utf8_lossy(&ran.stderr).into_owned(),
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
        let (out, _) = self.run(&["--write-out", "\n%{http_code}"], url, None)?;
        let s = String::from_utf8_lossy(&out).into_owned();
        let (body, code) = s.rsplit_once('\n').unwrap_or(("", &s));
        Self::status(code, url)?;
        Ok(body.to_string())
    }

    fn file(&self, url: &str, dest: &Path) -> Result<()> {
        let d = dest.to_string_lossy();
        let result = self
            .run(
                &["--write-out", "%{http_code}", "--output", &d],
                url,
                Some(dest),
            )
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
            None,
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
    use super::*;

    fn sh(script: &str) -> Child {
        Command::new("sh")
            .args(["-c", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn small_output_passes_through() {
        let r = supervise(sh("printf hello"), 1000, None, 1000);
        assert!(r.ok && !r.too_big);
        assert_eq!(r.stdout, b"hello");
    }

    #[test]
    fn output_past_the_cap_is_stopped() {
        let r = supervise(sh("head -c 200000 /dev/zero; sleep 30"), 1000, None, 1000);
        assert!(r.too_big && !r.ok);
    }

    #[test]
    fn a_file_growing_past_the_cap_is_stopped() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("big");
        let script = format!("head -c 200000 /dev/zero > '{}'; sleep 30", f.display());
        let r = supervise(sh(&script), 1000, Some(&f), 1000);
        assert!(r.too_big && !r.ok);
    }

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
