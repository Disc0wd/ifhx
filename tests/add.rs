//! `ifhx add` and what happens to added tools afterwards.
mod common;
use common::*;
use std::io::Write;

/// The smallest thing that passes the "is a program for this machine" check.
fn elf(tail: &str) -> Vec<u8> {
    let mut v = vec![0u8; 24];
    v[..4].copy_from_slice(b"\x7fELF");
    v[18] = 0x3E;
    v.extend_from_slice(tail.as_bytes());
    v
}

fn targz(path: &str, data: &[u8]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    h.set_mode(0o755);
    h.set_size(data.len() as u64);
    b.append_data(&mut h, path, data).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&b.into_inner().unwrap()).unwrap();
    gz.finish().unwrap()
}

fn allowed() -> Fixture {
    let mut f = Fixture::new();
    f.config.registry.allow_foreign = true;
    f
}

/// Publish a release as the GitHub API and download URLs would. `digest` = list a SHA-256 for it.
fn publish(f: &Fixture, repo: &str, tag: &str, asset: &str, data: &[u8], digest: bool) {
    let d = if digest {
        format!(r#","digest":"sha256:{}""#, sha256(data))
    } else {
        String::new()
    };
    f.fetcher.texts.lock().unwrap().insert(
        format!("https://api.github.com/repos/{repo}/releases/latest"),
        format!(r#"{{"tag_name":"{tag}","assets":[{{"name":"{asset}"{d}}}]}}"#),
    );
    f.fetcher.files.lock().unwrap().insert(
        format!("https://github.com/{repo}/releases/download/{tag}/{asset}"),
        data.to_vec(),
    );
    f.fetcher
        .tags
        .lock()
        .unwrap()
        .insert(repo.into(), tag.into());
}

/// Everything in the install directory except the running ifhx.
fn installed_files(f: &Fixture) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(f.bin())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "ifhx")
        .collect();
    v.sort();
    v
}

const REPO: &str = "someone/gizmo";
const ASSET: &str = "gizmo-linux-x86_64";

#[test]
fn adding_is_off_until_switched_on() {
    let f = Fixture::new();
    publish(&f, REPO, "v1.0.0", ASSET, &elf("1"), true);
    let (code, io) = f.yes("add someone/gizmo");
    assert_eq!(code, 1);
    assert!(
        io.err.contains("allow_foreign = true") && io.err.contains("does not vet"),
        "{}",
        io.err
    );
    assert_eq!(f.fetcher.fetches("someone"), 0, "nothing was contacted");
}

#[test]
fn adds_a_verified_program_and_remembers_where_it_came_from() {
    let f = allowed();
    publish(&f, REPO, "v1.0.0", ASSET, &elf("1"), true);
    let mut io = TestIo::answering(&["y"]);
    assert_eq!(
        f.run(&mut io, "add https://github.com/someone/gizmo"),
        0,
        "{}",
        io.err
    );
    assert!(
        io.out.contains("matches the SHA-256 digest") && io.out.contains("does not vet"),
        "{}",
        io.out
    );
    assert_eq!(std::fs::read(f.bin().join("gizmo")).unwrap(), elf("1"));
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(f.bin().join("gizmo"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    assert_eq!(installed_files(&f), ["gizmo"]);
    let r = &f.state().tools["gizmo"];
    assert_eq!(
        (r.method.as_str(), r.repo.as_deref(), r.verified),
        ("github", Some(REPO), true)
    );
    assert!(
        f.verifier.calls.lock().unwrap().is_empty(),
        "the IFHX key is not used"
    );
    let (_, list) = f.yes("list");
    assert!(list.out.contains("github"), "{}", list.out);
    let (_, info) = f.yes("info gizmo");
    assert!(info.out.contains(REPO), "{}", info.out);
    // Asking again does nothing.
    let (code, again) = f.yes("add someone/gizmo");
    assert_eq!(code, 0);
    assert!(again.out.contains("Already installed"), "{}", again.out);
}

#[test]
fn unpacks_the_program_from_a_tarball_checked_against_a_checksum_file() {
    let f = allowed();
    let data = targz("gizmo-1.0/gz", &elf("tarred"));
    let asset = "gizmo-1.0-x86_64-unknown-linux-musl.tar.gz";
    publish(&f, REPO, "v1.0", asset, &data, false);
    let sidecar = format!("{asset}.sha256");
    f.fetcher.texts.lock().unwrap().insert(
        format!("https://api.github.com/repos/{REPO}/releases/latest"),
        format!(r#"{{"tag_name":"v1.0","assets":[{{"name":"{asset}"}},{{"name":"{sidecar}"}}]}}"#),
    );
    f.fetcher.texts.lock().unwrap().insert(
        format!("https://github.com/{REPO}/releases/download/v1.0/{sidecar}"),
        format!("{}  {asset}\n", sha256(&data)),
    );
    let (code, io) = f.yes("add someone/gizmo");
    assert_eq!(code, 0, "{}", io.err);
    assert!(io.out.contains("checksum file"), "{}", io.out);
    assert_eq!(std::fs::read(f.bin().join("gz")).unwrap(), elf("tarred"));
    assert_eq!(installed_files(&f), ["gz"]);
}

#[test]
fn an_unverified_program_says_so_and_declining_leaves_nothing() {
    let f = allowed();
    publish(&f, REPO, "v1.0.0", ASSET, &elf("1"), false);
    let mut io = TestIo::answering(&["n"]);
    assert_eq!(f.run(&mut io, "add someone/gizmo"), 6);
    assert!(io.out.contains("UNVERIFIED"), "{}", io.out);
    assert!(installed_files(&f).is_empty(), "{:?}", installed_files(&f));
    assert!(f.state().tools.is_empty());
    // --yes does not count for something unverifiable, and neither does just pressing Enter.
    let (code, io) = f.yes("add someone/gizmo");
    assert_eq!(code, 5, "{}", io.err);
    assert!(io.err.contains("--yes does not apply"), "{}", io.err);
    let mut enter = TestIo::answering(&[""]);
    assert_eq!(f.run(&mut enter, "add someone/gizmo --yes"), 6);
    assert!(enter.asked[0].contains("[y/N]"), "{:?}", enter.asked);
    assert!(installed_files(&f).is_empty());
    let mut yes = TestIo::answering(&["y"]);
    assert_eq!(f.run(&mut yes, "add someone/gizmo"), 0, "{}", yes.err);
    assert_eq!(installed_files(&f), ["gizmo"]);
    // Updating it is just as unverifiable, so it asks again.
    publish(&f, REPO, "v1.1.0", ASSET, &elf("2"), false);
    let (code, io) = f.yes("update");
    assert_eq!(code, 1);
    assert!(io.err.contains("--yes does not apply"), "{}", io.err);
    assert_eq!(std::fs::read(f.bin().join("gizmo")).unwrap(), elf("1"));
    let mut both = TestIo::answering(&["y", "y"]);
    assert_eq!(f.run(&mut both, "update"), 0, "{}", both.err);
    assert_eq!(std::fs::read(f.bin().join("gizmo")).unwrap(), elf("2"));
}

#[test]
fn bad_downloads_and_unusable_releases_leave_nothing() {
    let f = allowed();
    publish(&f, REPO, "v1.0.0", ASSET, &elf("1"), true);
    f.fetcher.files.lock().unwrap().insert(
        format!("https://github.com/{REPO}/releases/download/v1.0.0/{ASSET}"),
        elf("tampered"),
    );
    let (code, io) = f.yes("add someone/gizmo");
    assert_eq!(code, 1);
    assert!(io.err.contains("does not match"), "{}", io.err);
    assert!(installed_files(&f).is_empty());

    publish(&f, REPO, "v1.0.0", ASSET, b"<html>oops</html>", true);
    let (_, io) = f.yes("add someone/gizmo");
    assert!(io.err.contains("not a program"), "{}", io.err);
    assert!(installed_files(&f).is_empty());

    publish(
        &f,
        REPO,
        "v1.0.0",
        "Gizmo-linux-x86_64.AppImage",
        b"x",
        true,
    );
    let (_, io) = f.yes("add someone/gizmo");
    assert!(io.err.contains("ifhai install someone/gizmo"), "{}", io.err);

    let (code, io) = f.yes("add nobody/nothing");
    assert_eq!(code, 3, "{}", io.err);
    assert_eq!(f.yes("add not-a-repo").0, 2);
    assert_eq!(f.yes("add ../etc").0, 2);
}

#[test]
fn added_programs_cannot_take_an_ifhx_name_or_replace_a_file() {
    let f = allowed();
    publish(&f, "someone/ifhai", "v1", ASSET, &elf("x"), true);
    let (code, io) = f.yes("add someone/ifhai");
    assert_eq!(code, 1);
    assert!(io.err.contains("name of an IFHX tool"), "{}", io.err);
    assert!(installed_files(&f).is_empty());

    let (_, io) = f.yes("add Disc0wd/ifhf");
    assert!(io.err.contains("ifhx install ifhf"), "{}", io.err);

    publish(&f, REPO, "v1.0.0", ASSET, &elf("1"), true);
    std::fs::write(f.bin().join("gizmo"), "mine").unwrap();
    let (code, io) = f.yes("add someone/gizmo");
    assert_eq!(code, 1);
    assert!(io.err.contains("not installed by ifhx"), "{}", io.err);
    assert_eq!(
        std::fs::read_to_string(f.bin().join("gizmo")).unwrap(),
        "mine"
    );
    assert_eq!(installed_files(&f), ["gizmo"]);
}

#[test]
fn a_program_that_would_shadow_another_command_is_refused() {
    let mut f = allowed();
    let other = f.dir.path().join("elsewhere");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("gizmo"), "the one already on PATH").unwrap();
    f.path_var = Some(format!("{}:{}", f.bin().display(), other.display()));
    publish(&f, REPO, "v1.0.0", ASSET, &elf("1"), true);
    let (code, io) = f.yes("add someone/gizmo");
    assert_eq!(code, 1);
    assert!(
        io.err.contains("already provides that command"),
        "{}",
        io.err
    );
    assert!(installed_files(&f).is_empty() && f.state().tools.is_empty());
    // System commands count even when PATH does not list their directory.
    f.path_var = Some(f.bin().display().to_string());
    publish(&f, "someone/sh", "v1", "sh-linux-x86_64", &elf("x"), true);
    let (code, io) = f.yes("add someone/sh");
    assert_eq!(code, 1);
    assert!(
        io.err.contains("already provides that command"),
        "{}",
        io.err
    );
    assert!(installed_files(&f).is_empty());
}

fn added() -> Fixture {
    let f = allowed();
    publish(&f, REPO, "v1.0.0", ASSET, &elf("1"), true);
    assert_eq!(f.yes("add someone/gizmo").0, 0);
    f
}

#[test]
fn update_moves_an_added_tool_to_the_new_release() {
    let f = added();
    publish(&f, REPO, "v1.1.0", ASSET, &elf("2"), true);
    let (code, io) = f.yes("update");
    assert_eq!(code, 0, "{}", io.err);
    assert_eq!(std::fs::read(f.bin().join("gizmo")).unwrap(), elf("2"));
    let r = &f.state().tools["gizmo"];
    assert_eq!((r.version.as_str(), r.verified), ("v1.1.0", true));
    assert_eq!(installed_files(&f), ["gizmo"]);
}

#[test]
fn update_refuses_a_release_that_stopped_publishing_checksums() {
    let f = added();
    publish(&f, REPO, "v1.1.0", ASSET, &elf("2"), false);
    let (code, io) = f.yes("update");
    assert_eq!(code, 1);
    assert!(io.err.contains("publishes no checksum"), "{}", io.err);
    assert_eq!(std::fs::read(f.bin().join("gizmo")).unwrap(), elf("1"));
    assert_eq!(installed_files(&f), ["gizmo"]);
}

#[test]
fn update_leaves_a_hand_modified_added_tool_and_never_builds_it() {
    let f = added();
    publish(&f, REPO, "v1.1.0", ASSET, &elf("2"), true);
    let (_, io) = f.yes("update --source");
    assert!(io.err.contains("never built from source"), "{}", io.err);
    assert!(f.builder.jobs.lock().unwrap().is_empty());
    std::fs::write(f.bin().join("gizmo"), "patched").unwrap();
    let (code, io) = f.yes("update");
    assert_ne!(code, 0);
    assert!(io.err.contains("was changed after"), "{}", io.err);
    assert_eq!(
        std::fs::read_to_string(f.bin().join("gizmo")).unwrap(),
        "patched"
    );
}

#[test]
fn switching_foreign_tools_off_stops_updating_them_but_removal_still_works() {
    let mut f = added();
    publish(&f, REPO, "v1.1.0", ASSET, &elf("2"), true);
    f.config.registry.allow_foreign = false;
    let (_, io) = f.yes("update");
    assert!(io.err.contains("allow_foreign is off"), "{}", io.err);
    assert_eq!(std::fs::read(f.bin().join("gizmo")).unwrap(), elf("1"));
    assert_eq!(f.yes("remove gizmo").0, 0);
    assert!(installed_files(&f).is_empty() && f.state().tools.is_empty());
}
