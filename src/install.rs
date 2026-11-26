//! Plan and perform one tool installation. A binary only ever lands in its final place after it
//! has been fully downloaded (or built) and verified, and the swap is a single `rename`, so a
//! failure at any point leaves the previously installed version untouched.

use crate::error::{Error, Result};
use crate::fetch::{Fetcher, valid_tag};
use crate::registry::Tool;
use crate::state::Record;
use crate::verify::{self, Verifier};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Release {
        asset_url: String,
        sum_url: String,
        sig_url: String,
        asset: String,
    },
    /// Build from source. `None` tag = the repository's default branch.
    Source { tag: Option<String> },
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub tool: Tool,
    /// Release tag, or "main" for an untagged source build.
    pub version: String,
    pub method: Method,
    pub dest: PathBuf,
    /// Why the plan is not the usual prebuilt download, if it is not.
    pub note: Option<String>,
}

impl Plan {
    pub fn method_name(&self) -> &'static str {
        match self.method {
            Method::Release { .. } => "release",
            Method::Source { .. } => "source",
        }
    }
}

pub struct SourceJob<'a> {
    pub repo: &'a str,
    pub tag: Option<&'a str>,
    pub bin: &'a str,
}

/// Builds a tool from source into `out`. Faked in tests.
pub trait Builder {
    fn build(&self, job: &SourceJob, out: &Path) -> Result<()>;
}

pub struct CargoBuilder;

fn have(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

impl Builder for CargoBuilder {
    fn build(&self, job: &SourceJob, out: &Path) -> Result<()> {
        if !have("git") || !have("cargo") {
            return Err(Error::Failed(
                "Building from source needs both git and cargo, and one of them was not found.\n\nInstall Rust (https://rustup.rs) and git, or wait for a prebuilt release of this tool."
                    .into(),
            ));
        }
        // A fresh, private (0700) directory that must not already exist, so another user cannot
        // pre-create it in the shared temp directory and swap files while we build.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let work = std::env::temp_dir().join(format!(
            "ifhx-build-{}-{nanos}-{}",
            std::process::id(),
            job.bin
        ));
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&work)
                .map_err(|e| {
                    Error::Failed(format!(
                        "Could not create a private build directory {}: {e}",
                        work.display()
                    ))
                })?;
        }
        let result = (|| -> Result<()> {
            let mut clone = Command::new("git");
            clone.args(["clone", "--quiet", "--depth", "1"]);
            if let Some(t) = job.tag {
                clone.args(["--branch", t]);
            }
            clone
                .arg(format!("https://github.com/{}.git", job.repo))
                .arg(&work);
            run(clone, "git clone")?;
            // The build command is fixed here. A registry never gets to name a program to run.
            let mut b = Command::new("cargo");
            b.args(["build", "--release", "--locked"])
                .current_dir(&work);
            run(b, "build")?;
            let built = work.join("target").join("release").join(job.bin);
            std::fs::copy(&built, out).map_err(|e| {
                Error::Failed(format!(
                    "The build finished but {} is missing: {e}",
                    built.display()
                ))
            })?;
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&work);
        result
    }
}

fn run(mut cmd: Command, what: &str) -> Result<()> {
    let status = cmd
        .stdin(Stdio::null())
        .status()
        .map_err(|e| Error::Failed(format!("Could not run {what}: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Failed(format!(
            "{what} failed ({status}); see the output above."
        )))
    }
}

/// A build from source runs the project's own build scripts, so it only ever happens when the
/// user asks for it with `--source`, never as a silent fallback.
fn no_binary(tool: &Tool, why: &str) -> Error {
    Error::Failed(format!(
        "{} {why}.\n\nTo build it from source instead, run: ifhx install {} --source\n(needs git and cargo; the build runs the project's own build scripts)",
        tool.name, tool.name
    ))
}

/// Decide how to install `tool`. `want` pins a tag; `force_source` always builds.
pub fn plan(
    tool: &Tool,
    fetcher: &dyn Fetcher,
    platform: &str,
    dest: PathBuf,
    want: Option<&str>,
    force_source: bool,
) -> Result<Plan> {
    let tag = match want {
        Some(t) => {
            if !valid_tag(t) {
                return Err(Error::Usage(format!("'{t}' is not a valid release tag.")));
            }
            Some(t.to_string())
        }
        None => fetcher.latest_tag(&tool.repo)?,
    };
    let source = |tag: Option<String>, note: Option<String>| Plan {
        version: tag.clone().unwrap_or_else(|| "main".into()),
        tool: tool.clone(),
        method: Method::Source { tag },
        dest: dest.clone(),
        note,
    };
    let Some(tag) = tag else {
        if force_source {
            return Ok(source(None, None));
        }
        return Err(no_binary(tool, "has no published release yet"));
    };
    if force_source {
        return Ok(source(Some(tag), None));
    }
    let asset = tool.asset_name(platform);
    let base = format!("https://github.com/{}/releases/download/{tag}", tool.repo);
    let sum_url = format!("{base}/{asset}.sha256");
    let sig_url = format!("{base}/{asset}.minisig");
    match fetcher.text(&sum_url) {
        Ok(_) => {
            // Refuse before asking the user anything: an unsigned release is never installed.
            match fetcher.text(&sig_url) {
                Ok(_) => {}
                Err(Error::NotFound(_)) => {
                    return Err(Error::Verify(format!(
                        "{} {tag} is not signed, so ifhx will not install it.\nIts maintainer has to sign the release (see the README), or you can build it yourself with: ifhx install {} --source",
                        tool.name, tool.name
                    )));
                }
                Err(e) => return Err(e),
            }
            Ok(Plan {
                tool: tool.clone(),
                version: tag,
                method: Method::Release {
                    asset_url: format!("{base}/{asset}"),
                    sum_url,
                    sig_url,
                    asset,
                },
                dest,
                note: None,
            })
        }
        Err(Error::NotFound(_)) => Err(no_binary(
            tool,
            &format!("has no prebuilt {platform} binary in release {tag}"),
        )),
        Err(e) => Err(e),
    }
}

/// The first whitespace-separated token of a `sha256sum` line, if it is a SHA-256 digest.
pub fn parse_checksum(text: &str) -> Option<String> {
    let tok = text.split_whitespace().next()?.to_ascii_lowercase();
    (tok.len() == 64 && tok.chars().all(|c| c.is_ascii_hexdigit())).then_some(tok)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub(crate) fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// `x86_64-linux` or `aarch64-linux`: the platform part of release asset names.
pub fn platform() -> Result<String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", a @ ("x86_64" | "aarch64")) => Ok(format!("{a}-linux")),
        (os, arch) => Err(Error::Failed(format!(
            "ifhx supports Linux on x86_64 and aarch64; this is {os} on {arch}."
        ))),
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn execute(
    plan: &Plan,
    fetcher: &dyn Fetcher,
    builder: &dyn Builder,
    verifier: &dyn Verifier,
) -> Result<Record> {
    let dir = plan.dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::Io(format!("Cannot create {}: {e}", dir.display())))?;
    // Same directory as the destination so the final rename is atomic.
    let tmp = dir.join(format!(".{}.ifhx-{}", plan.tool.bin, std::process::id()));
    let result = (|| -> Result<String> {
        match &plan.method {
            Method::Release {
                asset_url,
                sum_url,
                sig_url,
                asset,
            } => {
                let sums = fetcher.text(sum_url)?;
                let want = parse_checksum(&sums).ok_or_else(|| {
                    Error::Verify(format!(
                        "The checksum file for {asset} is not in sha256sum format."
                    ))
                })?;
                fetcher.file(asset_url, &tmp)?;
                let got = sha256_file(&tmp)?;
                if got != want {
                    return Err(Error::Verify(format!(
                        "The downloaded {asset} does not match its published checksum, so it was discarded.\n  expected {want}\n  got      {got}"
                    )));
                }
                // The checksum only catches corruption; the signature is what proves who published it.
                let sig = fetcher.text(sig_url)?;
                verifier.verify(&tmp, &sig, &verify::comment(asset, &plan.version))?;
            }
            Method::Source { tag } => {
                builder.build(
                    &SourceJob {
                        repo: &plan.tool.repo,
                        tag: tag.as_deref(),
                        bin: &plan.tool.bin,
                    },
                    &tmp,
                )?;
            }
        }
        make_executable(&tmp)?;
        let sha = sha256_file(&tmp)?;
        std::fs::rename(&tmp, &plan.dest)
            .map_err(|e| Error::Io(format!("Cannot install to {}: {e}", plan.dest.display())))?;
        Ok(sha)
    })();
    match result {
        Ok(sha256) => Ok(Record {
            version: plan.version.clone(),
            method: plan.method_name().into(),
            path: plan.dest.clone(),
            sha256,
            installed_at: now_secs(),
            ..Default::default()
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_lines() {
        let h = "a".repeat(64);
        assert_eq!(
            parse_checksum(&format!("{h}  ifhf-x86_64-linux\n")),
            Some(h.clone())
        );
        assert_eq!(parse_checksum(&format!("{}  f", h.to_uppercase())), Some(h));
        assert_eq!(parse_checksum("short  f"), None);
        assert_eq!(parse_checksum(&format!("{}zz  f", "a".repeat(62))), None);
        assert_eq!(parse_checksum(""), None);
    }

    #[test]
    fn sha256_matches_the_known_vector() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        std::fs::write(&p, "abc").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
