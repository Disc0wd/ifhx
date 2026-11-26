//! `ifhx add`: install a program from someone else's GitHub release.
//!
//! ifhx can check that a download is exactly the file the publisher released. It cannot tell you
//! the publisher is trustworthy, and the user is told so. A program is first *staged* (downloaded,
//! checked, unpacked into a hidden file beside its destination); only after the user confirms is it
//! renamed into place, so a refusal or a failure leaves nothing behind.

use crate::error::{Error, Result};
use crate::fetch::{FILE_MAX, Fetcher};
use crate::github::{self, Fmt, Release};
use crate::install::{make_executable, now_secs, sha256_file};
use crate::registry::ident;
use crate::state::Record;
use flate2::read::GzDecoder;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// A decompression bomb stops here, long before it can fill the disk.
const UNPACKED_MAX: u64 = 2 << 30;
/// Long names and PAX records are held in memory while read; real ones are a few hundred bytes.
const NAME_RECORD_MAX: u64 = 64 * 1024;

pub struct Staged {
    /// The command name: the executable's own name inside an archive, else the repository name.
    pub name: String,
    pub version: String,
    pub repo: String,
    pub asset: String,
    /// The release file name with the version replaced by `*`, to find the same kind of file later.
    pub pattern: String,
    /// What the SHA-256 was checked against, or None when the publisher gave nothing to check.
    pub checked: Option<&'static str>,
    pub sha256: String,
    tmp: PathBuf,
    kept: bool,
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.kept {
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn bad_archive(e: impl std::fmt::Display) -> Error {
    Error::Failed(format!("The archive could not be read: {e}"))
}

/// Refuse anything that is not a program for this machine, so a wrong-architecture file or an
/// error page that happened to be saved is never installed as a command.
fn check_program(path: &Path, arch: &str) -> Result<()> {
    let mut h = [0u8; 20];
    let n = std::fs::File::open(path)?.read(&mut h)?;
    if n >= 2 && &h[..2] == b"#!" {
        return Ok(());
    }
    if n < 20 || &h[..4] != b"\x7fELF" {
        return Err(Error::Failed(
            "The downloaded file is not a program, so it was not installed.".into(),
        ));
    }
    let machine = u16::from_le_bytes([h[18], h[19]]);
    let want = match arch {
        "x86_64" => 0x3E,
        "aarch64" => 0xB7,
        _ => return Ok(()),
    };
    if machine != want {
        return Err(Error::Failed(format!(
            "The downloaded program is built for a different processor than this {arch} machine, so it was not installed."
        )));
    }
    Ok(())
}

struct Member {
    index: usize,
    base: String,
    exec: bool,
    size: u64,
}

/// Pick one executable out of a .tar.gz and write just that file to `out`. Nothing else in the
/// archive is ever written: no directories, links or paths from the archive reach the disk.
fn extract(archive: &Path, repo_name: &str, out: &Path) -> Result<String> {
    let open = || -> Result<tar::Archive<_>> {
        let f = std::fs::File::open(archive)?;
        Ok(tar::Archive::new(GzDecoder::new(f).take(UNPACKED_MAX)))
    };
    // Look at the raw records first: the normal reader buffers a long-name record whole, so a tiny
    // archive declaring a gigabyte one could exhaust memory.
    for e in open()?.entries().map_err(bad_archive)?.raw(true) {
        let e = e.map_err(bad_archive)?;
        let t = e.header().entry_type();
        if (t.is_gnu_longname()
            || t.is_gnu_longlink()
            || t.is_pax_local_extensions()
            || t.is_pax_global_extensions())
            && e.size() > NAME_RECORD_MAX
        {
            return Err(Error::Failed(
                "The archive has an oversized name record, so it was not unpacked.".into(),
            ));
        }
    }
    let mut members = Vec::new();
    for (index, e) in open()?.entries().map_err(bad_archive)?.enumerate() {
        let e = e.map_err(bad_archive)?;
        // Only plain files. Symlinks, hard links and devices are ignored, as are paths that climb
        // out of the archive or start at the root.
        if !e.header().entry_type().is_file() {
            continue;
        }
        let Ok(path) = e.path() else { continue };
        if !path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        {
            continue;
        }
        let Some(base) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        members.push(Member {
            index,
            base: base.to_string(),
            exec: e.header().mode().unwrap_or(0) & 0o111 != 0,
            size: e.header().size().unwrap_or(0),
        });
    }
    let execs: Vec<&Member> = members
        .iter()
        .filter(|m| m.exec && m.size > 0 && m.size <= FILE_MAX)
        .collect();
    let chosen = match execs.as_slice() {
        [one] => *one,
        [] => {
            return Err(Error::Failed(
                "The archive has no executable file, so there is nothing to install.".into(),
            ));
        }
        many => {
            let named: Vec<&&Member> = many
                .iter()
                .filter(|m| m.base.eq_ignore_ascii_case(repo_name))
                .collect();
            match named.as_slice() {
                [one] => **one,
                _ => {
                    return Err(Error::Failed(format!(
                        "The archive has several executables and ifhx will not guess which you want:\n\n{}",
                        many.iter()
                            .take(12)
                            .map(|m| format!("  {}", m.base))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )));
                }
            }
        }
    };
    for (index, e) in open()?.entries().map_err(bad_archive)?.enumerate() {
        if index != chosen.index {
            continue;
        }
        let e = e.map_err(bad_archive)?;
        let mut f = std::fs::File::create(out)?;
        let n = std::io::copy(&mut e.take(FILE_MAX + 1), &mut f)?;
        if n > FILE_MAX {
            return Err(Error::Failed(
                "The program inside the archive is larger than ifhx accepts.".into(),
            ));
        }
        return Ok(chosen.base.clone());
    }
    Err(bad_archive("the file vanished between two reads"))
}

/// Download, verify and unpack the right file of `rel` into a hidden file inside `dir`.
pub fn prepare(
    fetcher: &dyn Fetcher,
    repo: &str,
    rel: &Release,
    arch: &str,
    pattern: Option<&str>,
    dir: &Path,
) -> Result<Staged> {
    let (asset, fmt) = github::pick(repo, &rel.assets, arch, pattern)?;
    let expected = github::expected_sha256(fetcher, repo, rel, asset)?;
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::Io(format!("Cannot create {}: {e}", dir.display())))?;
    let stem = repo.rsplit('/').next().unwrap_or(repo);
    let pid = std::process::id();
    let dl = dir.join(format!(".{stem}.ifhx-dl-{pid}"));
    let _dl_cleanup = Cleanup(dl.clone());
    fetcher.file(&github::download_url(repo, &rel.tag, &asset.name), &dl)?;
    let got = sha256_file(&dl)?;
    if let Some((want, _)) = &expected
        && *want != got
    {
        return Err(Error::Verify(format!(
            "The downloaded {} does not match the checksum its publisher gave, so it was discarded.\n  expected {want}\n  got      {got}",
            asset.name
        )));
    }
    let tmp = dir.join(format!(".{stem}.ifhx-{pid}"));
    let name = match fmt {
        Fmt::Raw => {
            std::fs::rename(&dl, &tmp)?;
            stem.to_string()
        }
        Fmt::TarGz => {
            let r = extract(&dl, stem, &tmp);
            if r.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            r?
        }
    };
    // From here on `staged` owns the hidden file and removes it on any early return.
    let mut staged = Staged {
        name,
        version: rel.tag.clone(),
        repo: repo.to_string(),
        asset: asset.name.clone(),
        pattern: github::pattern_from_asset(&asset.name, &rel.tag),
        checked: expected.map(|(_, how)| how),
        sha256: String::new(),
        tmp,
        kept: false,
    };
    if !ident(&staged.name) {
        return Err(Error::Failed(format!(
            "The program is called '{}', which is not a name ifhx will install.",
            staged.name.escape_debug()
        )));
    }
    check_program(&staged.tmp, arch)?;
    staged.sha256 = sha256_file(&staged.tmp)?;
    Ok(staged)
}

/// Move a staged program into place and describe it for the install record.
pub fn commit(mut s: Staged, dest: &Path) -> Result<Record> {
    make_executable(&s.tmp)?;
    std::fs::rename(&s.tmp, dest)
        .map_err(|e| Error::Io(format!("Cannot install to {}: {e}", dest.display())))?;
    s.kept = true;
    Ok(Record {
        version: s.version.clone(),
        method: "github".into(),
        path: dest.to_path_buf(),
        sha256: s.sha256.clone(),
        installed_at: now_secs(),
        repo: Some(s.repo.clone()),
        pattern: Some(s.pattern.clone()),
        verified: s.checked.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A gzip'd tar from (path bytes, mode, kind, contents).
    fn tarball(entries: &[(&[u8], u32, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (path, mode, kind, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.as_old_mut().name[..path.len()].copy_from_slice(path);
            h.set_mode(*mode);
            h.set_entry_type(*kind);
            h.set_size(data.len() as u64);
            if matches!(kind, tar::EntryType::Symlink) {
                h.set_link_name("/etc/passwd").unwrap();
                h.set_size(0);
            }
            h.set_cksum();
            b.append(
                &h,
                if matches!(kind, tar::EntryType::Symlink) {
                    &b""[..]
                } else {
                    data
                },
            )
            .unwrap();
        }
        let tar = b.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    fn run(entries: &[(&[u8], u32, tar::EntryType, &[u8])]) -> (Result<String>, String) {
        let d = tempfile::tempdir().unwrap();
        let (a, o) = (d.path().join("a.tar.gz"), d.path().join("out"));
        std::fs::write(&a, tarball(entries)).unwrap();
        let r = extract(&a, "tool", &o);
        let got = std::fs::read_to_string(&o).unwrap_or_default();
        let files = std::fs::read_dir(d.path()).unwrap().count();
        assert_eq!(
            files,
            if r.is_ok() {
                2
            } else {
                1 + o.exists() as usize
            },
            "nothing else is written"
        );
        (r, got)
    }

    use tar::EntryType::{Regular, Symlink};

    #[test]
    fn takes_the_one_executable_and_writes_only_that() {
        let (r, got) = run(&[
            (b"tool-1.0/README.md", 0o644, Regular, b"docs"),
            (b"tool-1.0/rg", 0o755, Regular, b"the program"),
        ]);
        assert_eq!(r.unwrap(), "rg");
        assert_eq!(got, "the program");
    }

    #[test]
    fn several_executables_need_the_repository_name_or_an_error() {
        let (r, got) = run(&[
            (b"bin/tool", 0o755, Regular, b"main"),
            (b"bin/helper", 0o755, Regular, b"other"),
        ]);
        assert_eq!((r.unwrap().as_str(), got.as_str()), ("tool", "main"));
        let (r, _) = run(&[(b"a", 0o755, Regular, b"1"), (b"b", 0o755, Regular, b"2")]);
        assert!(r.unwrap_err().to_string().contains("will not guess"));
    }

    #[test]
    fn links_and_escaping_paths_are_ignored() {
        // Only a symlink and path-traversal entries: there is nothing legitimate to install.
        let (r, _) = run(&[
            (b"tool", 0o755, Symlink, b""),
            (b"../evil", 0o755, Regular, b"x"),
            (b"/abs/evil", 0o755, Regular, b"x"),
        ]);
        assert!(r.unwrap_err().to_string().contains("no executable"));
        // A legitimate entry next to hostile ones is still found, and only it is written.
        let (r, got) = run(&[
            (b"../evil", 0o755, Regular, b"EVIL"),
            (b"tool", 0o755, Symlink, b""),
            (b"ok/tool", 0o755, Regular, b"fine"),
        ]);
        assert_eq!((r.unwrap().as_str(), got.as_str()), ("tool", "fine"));
    }

    #[test]
    fn an_archive_without_a_program_or_a_broken_one_is_refused() {
        let (r, _) = run(&[(b"README", 0o644, Regular, b"docs")]);
        assert!(r.is_err());
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.tar.gz");
        std::fs::write(&a, b"not gzip at all").unwrap();
        assert!(extract(&a, "tool", &d.path().join("o")).is_err());
    }

    #[test]
    fn a_huge_name_record_is_refused() {
        let name = vec![b'a'; 1 << 20];
        let (r, _) = run(&[
            (b"././@LongLink", 0o644, tar::EntryType::GNULongName, &name),
            (b"tool", 0o755, Regular, b"x"),
        ]);
        assert!(r.unwrap_err().to_string().contains("oversized name record"));
    }

    /// Recompute a header's checksum so a mutated header is parsed rather than rejected outright.
    fn fix_checksum(block: &mut [u8]) {
        block[148..156].fill(b' ');
        let sum: u32 = block.iter().map(|&b| b as u32).sum();
        block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    }

    /// Thousands of damaged archives: whatever is in them, no panic and nothing written but `out`.
    #[test]
    fn corrupted_archives_never_panic_or_write_elsewhere() {
        let mut tar_bytes = Vec::new();
        GzDecoder::new(
            &tarball(&[
                (b"dir/readme", 0o644, Regular, b"docs"),
                (
                    b"././@LongLink",
                    0o644,
                    tar::EntryType::GNULongName,
                    b"dir/long\0",
                ),
                (b"dir/tool", 0o755, Regular, &[7u8; 900]),
                (b"link", 0o755, Symlink, b""),
            ])[..],
        )
        .read_to_end(&mut tar_bytes)
        .unwrap();
        let headers: Vec<usize> = (0..tar_bytes.len())
            .step_by(512)
            .filter(|&i| tar_bytes.get(i + 257..i + 262) == Some(b"ustar"))
            .collect();
        assert!(headers.len() >= 4);
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as usize
        };
        let d = tempfile::tempdir().unwrap();
        let (a, out) = (d.path().join("a.tar.gz"), d.path().join("out"));
        let mut parsed = 0;
        for i in 0..4000 {
            let mut t = tar_bytes.clone();
            for _ in 0..1 + rand(8) {
                // Mostly aim at header fields, where the parsing happens.
                let h = headers[rand(headers.len())];
                let at = if rand(5) == 0 {
                    rand(t.len())
                } else {
                    h + rand(512)
                };
                t[at] = rand(256) as u8;
            }
            for &h in &headers {
                fix_checksum(&mut t[h..h + 512]);
            }
            if rand(10) == 0 {
                t.truncate(rand(t.len()));
            }
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gz.write_all(&t).unwrap();
            let mut z = gz.finish().unwrap();
            if i % 4 == 0 {
                let at = rand(z.len());
                z[at] ^= 1 + rand(255) as u8;
            }
            std::fs::write(&a, &z).unwrap();
            parsed += extract(&a, "tool", &out).is_ok() as u32;
            let _ = std::fs::remove_file(&out);
            assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1, "round {i}");
        }
        // The damage must be shallow often enough to exercise the code behind the parser.
        assert!(parsed > 400 && parsed < 4000, "{parsed}");
    }

    #[test]
    fn only_programs_for_this_machine_pass() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("p");
        let elf = |machine: u8| {
            let mut v = vec![0u8; 64];
            v[..4].copy_from_slice(b"\x7fELF");
            v[18] = machine;
            v
        };
        std::fs::write(&p, elf(0x3E)).unwrap();
        check_program(&p, "x86_64").unwrap();
        assert!(check_program(&p, "aarch64").is_err());
        std::fs::write(&p, "#!/bin/sh\necho hi\n").unwrap();
        check_program(&p, "x86_64").unwrap();
        std::fs::write(&p, "<html>404</html>").unwrap();
        assert!(check_program(&p, "x86_64").is_err());
        std::fs::write(&p, "").unwrap();
        assert!(check_program(&p, "x86_64").is_err());
    }
}
