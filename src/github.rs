//! GitHub release metadata, and choosing the right file to install for `ifhx add`.
//!
//! Everything the API says is untrusted text. Tags and file names are checked before they are used
//! or shown, and download URLs are built here from the repository, tag and name, never copied from
//! the response.

use crate::error::{Error, Result};
use crate::fetch::{Fetcher, valid_tag};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    /// `sha256:<hex>` as GitHub computed it, when it did.
    pub digest: Option<String>,
}

#[derive(Debug)]
pub struct Release {
    pub tag: String,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

/// A file name we are willing to put in a URL, a path and a terminal.
fn safe_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 200
        && !n.starts_with(['-', '.'])
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
}

pub fn parse_release(json: &str) -> Result<Release> {
    let raw: RawRelease = serde_json::from_str(json).map_err(|e| {
        Error::Network(format!(
            "GitHub sent a release description ifhx could not read: {e}"
        ))
    })?;
    if !valid_tag(&raw.tag_name) {
        return Err(Error::Failed(
            "The latest release has a tag name ifhx will not use.".into(),
        ));
    }
    Ok(Release {
        tag: raw.tag_name,
        // Names that are not plain are dropped: a hostile release cannot smuggle one into a path.
        assets: raw
            .assets
            .into_iter()
            .filter(|a| safe_name(&a.name))
            .map(|a| Asset {
                name: a.name,
                digest: a.digest,
            })
            .collect(),
    })
}

pub fn latest_release(fetcher: &dyn Fetcher, repo: &str) -> Result<Release> {
    match fetcher.text(&format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    )) {
        Ok(j) => parse_release(&j),
        Err(Error::NotFound(_)) => Err(Error::NotFound(format!(
            "{repo} has no published releases (or does not exist)."
        ))),
        Err(Error::Network(m)) if m.contains("HTTP 403") || m.contains("HTTP 429") => {
            Err(Error::Network(format!(
                "GitHub is limiting requests from your network right now.\n{m}\nTry again in a while."
            )))
        }
        Err(e) => Err(e),
    }
}

/// The download URL of one file of a release.
pub fn download_url(repo: &str, tag: &str, name: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{name}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fmt {
    Raw,
    TarGz,
}

enum Class {
    Skip,
    AppImage,
    Unsupported,
    Usable(Fmt),
}

const OTHER_OS: &[&str] = &[
    "darwin", "macos", "osx", "windows", "win32", "win64", "freebsd", "openbsd", "netbsd",
    "android", "apple", "ios",
];
const NOT_A_PROGRAM: &[&str] = &[
    ".sha256",
    ".sha256sum",
    ".sha512",
    ".md5",
    ".sig",
    ".asc",
    ".minisig",
    ".pem",
    ".sbom",
    ".json",
    ".jsonl",
    ".txt",
    ".md",
    ".sh",
    ".ps1",
    ".pub",
    ".crt",
    ".deb",
    ".rpm",
    ".apk",
    ".msi",
    ".exe",
    ".dmg",
    ".pkg",
    ".snap",
    ".flatpak",
    ".whl",
    ".jar",
];
const UNSUPPORTED: &[&str] = &[
    ".zip", ".tar.xz", ".txz", ".tar.zst", ".tar.bz2", ".tbz", ".tar", ".7z", ".xz", ".gz", ".bz2",
    ".zst",
];

const ARCHES: &[(&str, &[&str])] = &[
    ("x86_64", &["x86_64", "amd64", "x64"]),
    ("aarch64", &["aarch64", "arm64"]),
    ("arm", &["armhf", "armv7"]),
    ("x86", &["i386", "i686", "x86_32"]),
];

fn arch_tokens(arch: &str) -> &'static [&'static str] {
    ARCHES
        .iter()
        .find(|(a, _)| *a == arch)
        .map(|(_, t)| *t)
        .unwrap_or(&[])
}

fn mentions(name: &str, toks: &[&str]) -> bool {
    toks.iter().any(|t| name.contains(t))
}

fn mentions_arch(name: &str, toks: &[&str]) -> bool {
    // The `x64` inside `linux64` is not an architecture name.
    mentions(&name.to_ascii_lowercase().replace("linux64", "linux"), toks)
}

/// Does the name end in something that looks like a file extension (not a version or platform)?
fn has_ext(n: &str) -> bool {
    n.rsplit_once('.').is_some_and(|(_, e)| {
        !e.is_empty()
            && e.len() <= 5
            && e.chars().all(|c| c.is_ascii_alphanumeric())
            && !e.chars().all(|c| c.is_ascii_digit())
    })
}

// shortcut: only raw executables and .tar.gz are unpacked; .zip, .tar.xz and friends are refused
// with a clear message. Add them when a real tool needs it.
fn classify(name: &str) -> Class {
    let n = name.to_ascii_lowercase();
    // An AppImage is for Linux whether or not its name says so.
    if n.ends_with(".appimage") {
        return Class::AppImage;
    }
    if !n.contains("linux")
        || mentions(&n, OTHER_OS)
        || n.contains("sha256sums")
        || n.contains("checksums")
        || NOT_A_PROGRAM.iter().any(|e| n.ends_with(e))
    {
        return Class::Skip;
    }
    if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        return Class::Usable(Fmt::TarGz);
    }
    if UNSUPPORTED.iter().any(|e| n.ends_with(e)) {
        return Class::Unsupported;
    }
    if has_ext(&n) && !n.ends_with(".bin") {
        return Class::Skip;
    }
    Class::Usable(Fmt::Raw)
}

/// Pick the one file of `assets` to install on `arch`. Several equally good candidates is an error
/// that lists them: guessing which build someone wants is how they end up with the wrong one.
/// `pattern` (an update) insists on the same kind of file that was installed before.
pub fn pick<'a>(
    repo: &str,
    assets: &'a [Asset],
    arch: &str,
    pattern: Option<&str>,
) -> Result<(&'a Asset, Fmt)> {
    let mut pool: Vec<(&Asset, Fmt)> = Vec::new();
    let (mut appimages, mut unsupported) = (Vec::new(), Vec::new());
    for a in assets {
        match classify(&a.name) {
            Class::Usable(f) => pool.push((a, f)),
            Class::AppImage => appimages.push(a.name.as_str()),
            Class::Unsupported => unsupported.push(a.name.as_str()),
            Class::Skip => {}
        }
    }
    let host = arch_tokens(arch);
    let any: Vec<&str> = ARCHES.iter().flat_map(|(_, t)| t.iter().copied()).collect();
    let tagged: Vec<_> = pool
        .iter()
        .copied()
        .filter(|(a, _)| mentions_arch(&a.name, host))
        .collect();
    pool = if tagged.is_empty() {
        pool.into_iter()
            .filter(|(a, _)| !mentions_arch(&a.name, &any))
            .collect()
    } else {
        tagged
    };
    if let Some(p) = pattern {
        pool.retain(|(a, _)| glob(p, &a.name));
        if pool.is_empty() {
            return Err(Error::Failed(format!(
                "This release has no file matching {p}, the kind of file that was installed before.\nIf the project changed how it names releases, remove the tool and add it again."
            )));
        }
    }
    if pool.len() > 1
        && pool
            .iter()
            .any(|(a, _)| a.name.to_ascii_lowercase().contains("musl"))
    {
        pool.retain(|(a, _)| a.name.to_ascii_lowercase().contains("musl"));
    }
    match pool.as_slice() {
        [(a, f)] => Ok((a, *f)),
        [] if !appimages.is_empty() => Err(Error::Failed(format!(
            "{repo} only publishes AppImages for Linux ({}).\nifhai installs those: ifhai install {repo}",
            appimages.join(", ")
        ))),
        [] if !unsupported.is_empty() => Err(Error::Failed(format!(
            "{repo} publishes its Linux builds as {}, which ifhx cannot unpack yet.\nifhx supports plain executables and .tar.gz files.",
            unsupported.join(", ")
        ))),
        [] => Err(Error::NotFound(format!(
            "This release of {repo} has no Linux {arch} program ifhx can install."
        ))),
        many => Err(Error::Failed(format!(
            "This release of {repo} has several files that could be the right one:\n\n{}\n\nifhx will not guess. Ask the project which one is meant for you.",
            many.iter()
                .map(|(a, _)| format!("  {}", a.name))
                .collect::<Vec<_>>()
                .join("\n")
        ))),
    }
}

/// `*` matches any run of characters.
pub fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || text.len() < first.len() + last.len() {
        return false;
    }
    let mut pos = first.len();
    let end = text.len() - last.len();
    if !text.ends_with(last) {
        return false;
    }
    for mid in &parts[1..parts.len() - 1] {
        match text[pos..end].find(mid) {
            Some(i) => pos += i + mid.len(),
            None => return false,
        }
    }
    true
}

/// `foo-1.2.3-linux-x86_64.tar.gz` + tag `v1.2.3` -> `foo-*-linux-x86_64.tar.gz`, so the same
/// variant is found in the next release.
pub fn pattern_from_asset(name: &str, tag: &str) -> String {
    let bare = tag.trim_start_matches(['v', 'V']);
    if bare.is_empty() || !name.contains(bare) {
        return name.to_string();
    }
    name.replacen(&format!("v{bare}"), "*", 1)
        .replacen(&format!("V{bare}"), "*", 1)
        .replacen(bare, "*", 1)
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The SHA-256 the publisher gives for `asset`, and where it came from: GitHub's own digest first,
/// then a checksum file in the same release. None = nothing was published, so it is unverified.
pub fn expected_sha256(
    fetcher: &dyn Fetcher,
    repo: &str,
    rel: &Release,
    asset: &Asset,
) -> Result<Option<(String, &'static str)>> {
    if let Some(h) = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|h| is_sha256(h))
    {
        return Ok(Some((
            h.to_ascii_lowercase(),
            "the SHA-256 digest GitHub recorded for the file",
        )));
    }
    let own = [
        format!("{}.sha256", asset.name),
        format!("{}.sha256sum", asset.name),
    ];
    let shared = |n: &str| {
        let l = n.to_ascii_lowercase();
        l.starts_with("sha256sum") || l.starts_with("checksums") || l == "sha256.txt"
    };
    let Some(side) = rel
        .assets
        .iter()
        .find(|a| own.contains(&a.name))
        .or_else(|| rel.assets.iter().find(|a| shared(&a.name)))
    else {
        return Ok(None);
    };
    let text = match fetcher.text(&download_url(repo, &rel.tag, &side.name)) {
        Ok(t) => t,
        Err(Error::NotFound(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(sum_for(&text, &asset.name).map(|h| (h, "the checksum file published in the release")))
}

/// From `sha256sum`-style text: the digest on the line for `name`, or the only digest in a
/// one-line sidecar file.
pub fn sum_for(text: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    for l in &lines {
        let mut p = l.split_whitespace();
        if let (Some(h), Some(f)) = (p.next(), p.next())
            && is_sha256(h)
            && f.trim_start_matches('*') == name
        {
            return Some(h.to_ascii_lowercase());
        }
    }
    match lines.as_slice() {
        [one] => one
            .split_whitespace()
            .next()
            .filter(|h| is_sha256(h))
            .map(str::to_ascii_lowercase),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets(names: &[&str]) -> Vec<Asset> {
        names
            .iter()
            .map(|n| Asset {
                name: n.to_string(),
                digest: None,
            })
            .collect()
    }

    fn chosen(names: &[&str]) -> Result<String> {
        let a = assets(names);
        pick("o/r", &a, "x86_64", None).map(|(a, _)| a.name.clone())
    }

    #[test]
    fn picks_the_linux_build_for_this_machine() {
        assert_eq!(
            chosen(&[
                "tool-1.0-x86_64-unknown-linux-musl.tar.gz",
                "tool-1.0-aarch64-unknown-linux-musl.tar.gz",
                "tool-1.0-x86_64-apple-darwin.tar.gz",
                "tool-1.0-x86_64-pc-windows-msvc.zip",
                "tool-1.0-x86_64-unknown-linux-musl.tar.gz.sha256",
                "tool_1.0_amd64.deb",
            ])
            .unwrap(),
            "tool-1.0-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            chosen(&["tool_1.0_linux_amd64.tar.gz", "tool_1.0_linux_arm64.tar.gz"]).unwrap(),
            "tool_1.0_linux_amd64.tar.gz"
        );
        assert_eq!(
            chosen(&["jq-linux-amd64", "jq-linux64", "jq-linux-arm64"]).unwrap(),
            "jq-linux-amd64"
        );
        assert_eq!(chosen(&["jq-linux64"]).unwrap(), "jq-linux64");
        assert_eq!(
            chosen(&["tool-1.2.3-linux-amd64", "tool-1.2.3-darwin-amd64"]).unwrap(),
            "tool-1.2.3-linux-amd64"
        );
    }

    #[test]
    fn musl_wins_a_tie_and_other_ties_are_listed_not_guessed() {
        assert_eq!(
            chosen(&["t-linux-x64-gnu.tar.gz", "t-linux-x64-musl.tar.gz"]).unwrap(),
            "t-linux-x64-musl.tar.gz"
        );
        let e = chosen(&["t-linux-x64-a.tar.gz", "t-linux-x64-b.tar.gz"]).unwrap_err();
        assert!(
            e.to_string().contains("t-linux-x64-a") && e.to_string().contains("will not guess"),
            "{e}"
        );
    }

    #[test]
    fn unusable_releases_say_why() {
        let e = chosen(&["Tool-x86_64.AppImage", "tool-linux.sha256"]).unwrap_err();
        assert!(e.to_string().contains("ifhai install o/r"), "{e}");
        let e = chosen(&["tool-linux-x86_64.zip", "tool-linux-x86_64.tar.xz"]).unwrap_err();
        assert!(e.to_string().contains("cannot unpack"), "{e}");
        let e = chosen(&["tool-darwin-arm64.tar.gz"]).unwrap_err();
        assert!(e.to_string().contains("no Linux x86_64 program"), "{e}");
        assert!(chosen(&[]).is_err());
    }

    #[test]
    fn an_update_keeps_to_the_kind_of_file_that_was_installed() {
        let a = assets(&["t-2.0-linux-x64-gnu.tar.gz", "t-2.0-linux-x64-musl.tar.gz"]);
        let (got, _) = pick("o/r", &a, "x86_64", Some("t-*-linux-x64-gnu.tar.gz")).unwrap();
        assert_eq!(got.name, "t-2.0-linux-x64-gnu.tar.gz");
        assert!(pick("o/r", &a, "x86_64", Some("other-*.tar.gz")).is_err());
    }

    #[test]
    fn globs_and_patterns() {
        assert!(glob("a-*-b", "a-1.2-b") && glob("*", "x") && glob("a*b*c", "a1b2c"));
        assert!(!glob("a-*-b", "a-1.2-c") && !glob("a*b", "ab-x") && glob("exact", "exact"));
        assert!(!glob("exact", "exact2") && !glob("a*a", "a"));
        assert_eq!(
            pattern_from_asset("foo-1.2.3-linux.tar.gz", "v1.2.3"),
            "foo-*-linux.tar.gz"
        );
        assert_eq!(pattern_from_asset("foo-linux", "v1.2.3"), "foo-linux");
    }

    #[test]
    fn hostile_release_json_is_cleaned() {
        let r = parse_release(
            r#"{"tag_name":"v1.0","assets":[
                {"name":"ok-linux-x64","digest":"sha256:ab"},
                {"name":"../../etc/passwd"},
                {"name":"x\u001b[2Jy"},
                {"name":"-rf"}]}"#,
        )
        .unwrap();
        assert_eq!(r.assets.len(), 1);
        assert_eq!(r.assets[0].name, "ok-linux-x64");
        assert!(parse_release(r#"{"tag_name":"v1;rm","assets":[]}"#).is_err());
        assert!(parse_release("<html>").is_err());
    }

    #[test]
    fn checksum_lines() {
        let h = "a".repeat(64);
        assert_eq!(
            sum_for(&format!("{h}  f.tar.gz\n"), "f.tar.gz"),
            Some(h.clone())
        );
        assert_eq!(
            sum_for(&format!("{h} *f\n{}  g\n", "b".repeat(64)), "f"),
            Some(h.clone())
        );
        assert_eq!(sum_for(&format!("{h}\n"), "anything"), Some(h));
        assert_eq!(sum_for("junk", "f"), None);
    }
}
