mod common;
use common::*;
use ifhx::registry::BUILTIN;

fn strip(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            for d in it.by_ref() {
                if d == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn ifhf_installed(f: &Fixture, tag: &str, content: &str) {
    f.release("Disc0wd/ifhf", "ifhf", tag, content);
    let (code, io) = f.yes("install ifhf");
    assert_eq!(code, 0, "{}", io.err);
}

// ---- install --------------------------------------------------------------------------

#[test]
fn installs_a_verified_prebuilt_binary() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "binary-v0.1.0");
    let mut io = TestIo::answering(&["y"]);
    assert_eq!(f.run(&mut io, "install ifhf"), 0, "{}", io.err);
    let dest = f.bin().join("ifhf");
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "binary-v0.1.0");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(
        io.out.contains("Version:")
            && io.out.contains("0.1.0")
            && io
                .out
                .contains("prebuilt binary (x86_64-linux), checksum verified"),
        "{}",
        io.out
    );
    assert!(io.out.contains(&dest.display().to_string()));
    assert!(io.out.contains("Installed: ifhf 0.1.0"), "{}", io.out);
    assert_eq!(io.asked, vec!["Install ifhf? [Y/n] "]);
    let rec = &f.state().tools["ifhf"];
    assert_eq!(
        (rec.version.as_str(), rec.method.as_str()),
        ("v0.1.0", "release")
    );
    assert_eq!(rec.sha256, sha256(b"binary-v0.1.0"));
    assert_eq!(rec.path, dest);
    assert!(
        f.builder.jobs.lock().unwrap().is_empty(),
        "no source build when a binary exists"
    );
}

#[test]
fn a_corrupted_download_is_rejected_and_leaves_nothing_behind() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "good");
    f.fetcher.files.lock().unwrap().insert(
        "https://github.com/Disc0wd/ifhf/releases/download/v0.1.0/ifhf-x86_64-linux".into(),
        b"tampered".to_vec(),
    );
    let (code, io) = f.yes("install ifhf");
    assert_eq!(code, 1);
    assert!(
        io.err.contains("does not match its published checksum") && io.err.contains("discarded"),
        "{}",
        io.err
    );
    assert!(!f.bin().join("ifhf").exists());
    let leftovers: Vec<_> = std::fs::read_dir(f.bin())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["ifhx"], "no temp files: {leftovers:?}");
    assert!(f.state().tools.is_empty(), "nothing recorded");
}

#[test]
fn a_malformed_checksum_file_is_rejected() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    f.fetcher.texts.lock().unwrap().insert(
        "https://github.com/Disc0wd/ifhf/releases/download/v0.1.0/ifhf-x86_64-linux.sha256".into(),
        "<html>not a checksum</html>".into(),
    );
    let (code, io) = f.yes("install ifhf");
    assert_eq!(code, 1);
    assert!(io.err.contains("sha256sum format"), "{}", io.err);
    assert!(!f.bin().join("ifhf").exists());
}

#[test]
fn a_release_without_a_binary_is_never_built_silently() {
    let f = Fixture::new();
    f.fetcher
        .tags
        .lock()
        .unwrap()
        .insert("Disc0wd/ifhf".into(), "v0.2.0".into()); // release exists, no assets
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "install ifhf --yes"), 1, "{}", io.out);
    assert!(
        io.err.contains("no prebuilt x86_64-linux binary") && io.err.contains("--source"),
        "{}",
        io.err
    );
    assert!(
        f.builder.jobs.lock().unwrap().is_empty(),
        "nothing was built"
    );
    assert!(!f.bin().join("ifhf").exists());
}

#[test]
fn source_flag_builds_a_release_that_has_no_binary() {
    let f = Fixture::new();
    f.fetcher
        .tags
        .lock()
        .unwrap()
        .insert("Disc0wd/ifhf".into(), "v0.2.0".into());
    let (code, io) = f.yes("install ifhf --source");
    assert_eq!(code, 0, "{}", io.err);
    assert_eq!(
        f.builder.jobs.lock().unwrap().clone(),
        vec!["Disc0wd/ifhf v0.2.0"]
    );
    let rec = &f.state().tools["ifhf"];
    assert_eq!(
        (rec.version.as_str(), rec.method.as_str()),
        ("v0.2.0", "source")
    );
}

#[test]
fn a_tool_with_no_releases_needs_source_to_be_asked_for() {
    let f = Fixture::new();
    let (code, io) = f.yes("install ifhf");
    assert_eq!(code, 1);
    assert!(io.err.contains("no published release yet"), "{}", io.err);
    assert!(f.builder.jobs.lock().unwrap().is_empty());
    let (code, io) = f.yes("install ifhf --source");
    assert_eq!(code, 0, "{}", io.err);
    assert_eq!(
        f.builder.jobs.lock().unwrap().clone(),
        vec!["Disc0wd/ifhf HEAD"]
    );
    assert_eq!(f.state().tools["ifhf"].version, "main");
}

#[test]
fn source_flag_forces_a_build_even_when_a_binary_exists() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "prebuilt");
    let (code, _) = f.yes("install ifhf --source");
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read_to_string(f.bin().join("ifhf")).unwrap(),
        "built-from-source:v0.1.0"
    );
    assert_eq!(f.fetcher.fetches("file "), 0, "no binary downloaded");
}

#[test]
fn missing_build_tools_are_explained() {
    let f = Fixture::new();
    *f.builder.fail.lock().unwrap() = true;
    let (code, io) = f.yes("install ifhf --source");
    assert_eq!(code, 1);
    assert!(
        io.err.contains("Could not install ifhf") && io.err.contains("git and cargo"),
        "{}",
        io.err
    );
    assert!(!f.bin().join("ifhf").exists());
}

#[test]
fn tag_pins_a_specific_release() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "old");
    f.fetcher
        .tags
        .lock()
        .unwrap()
        .insert("Disc0wd/ifhf".into(), "v0.3.0".into()); // latest differs
    // v0.1.0's files exist; ask for it explicitly.
    let (code, io) = f.yes("install ifhf --tag v0.1.0");
    assert_eq!(code, 0, "{}", io.err);
    assert_eq!(
        std::fs::read_to_string(f.bin().join("ifhf")).unwrap(),
        "old"
    );
    assert_eq!(f.state().tools["ifhf"].version, "v0.1.0");
    let (code, io) = f.yes("install ifhf --tag ../../evil");
    assert_eq!(
        code, 0,
        "already installed is skipped before the tag is used: {}",
        io.err
    );
    let g = Fixture::new();
    let (code, io) = g.yes("install ifhf --tag ../../evil");
    assert_eq!(code, 2, "{}", io.err);
    assert!(io.err.contains("not a valid release tag"));
}

#[test]
fn unknown_tool_and_already_installed() {
    let f = Fixture::new();
    let (code, io) = f.yes("install nope");
    assert_eq!(code, 3);
    assert!(
        io.err.contains("no tool called 'nope'") && io.err.contains("ifhx available"),
        "{}",
        io.err
    );
    ifhf_installed(&f, "v0.1.0", "bin");
    let before = f.fetcher.calls().len();
    let (code, io) = f.yes("install ifhf");
    assert_eq!(code, 0);
    assert!(
        io.out.contains("Already installed: ifhf 0.1.0") && io.out.contains("ifhx update ifhf"),
        "{}",
        io.out
    );
    assert_eq!(f.fetcher.fetches("file "), 1, "nothing downloaded again");
    assert!(f.fetcher.calls().len() >= before);
    let (code, io) = f.yes("install ifhx");
    assert_eq!(code, 0);
    assert!(
        io.out.contains("Already installed: ifhx"),
        "ifhx is the running program: {}",
        io.out
    );
}

#[test]
fn nothing_installs_if_any_requested_tool_is_unknown() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    let (code, io) = f.yes("install ifhf nope");
    assert_eq!(code, 3);
    assert!(io.err.contains("'nope'"));
    assert!(!f.bin().join("ifhf").exists());
}

#[test]
fn confirmation_rules() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "install ifhf"), 5);
    assert!(io.err.contains("--yes") && io.asked.is_empty() && !f.bin().join("ifhf").exists());
    let mut io = TestIo::answering(&["n"]);
    assert_eq!(f.run(&mut io, "install ifhf"), 6);
    assert_eq!(io.err, "ifhx: Cancelled.\n");
    assert!(!f.bin().join("ifhf").exists());
    let mut io = TestIo::answering(&[""]);
    assert_eq!(
        f.run(&mut io, "install ifhf"),
        0,
        "enter accepts the default"
    );
    let g = Fixture::new();
    g.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    let mut cfg = g;
    cfg.config = ifhx::config::Config::parse("[general]\nconfirm = false\n", "t").unwrap();
    let mut io = TestIo::non_interactive();
    assert_eq!(
        cfg.run(&mut io, "install ifhf"),
        0,
        "config can skip the prompt: {}",
        io.err
    );
}

#[test]
fn network_failure_is_reported_clearly_and_changes_nothing() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    f.fetcher.down.lock().unwrap().push("Disc0wd/ifhf".into());
    let (code, io) = f.yes("install ifhf");
    assert_eq!(code, 1);
    assert!(
        io.err.contains("Could not reach") || io.err.contains("network"),
        "{}",
        io.err
    );
    assert!(!f.bin().join("ifhf").exists() && f.state().tools.is_empty());
}

#[test]
fn warns_when_the_install_directory_is_not_on_path() {
    let mut f = Fixture::new();
    f.path_var = Some("/usr/bin:/bin".into());
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    let (code, io) = f.yes("install ifhf");
    assert_eq!(code, 0);
    assert!(
        io.err.contains("is not on your PATH") && io.err.contains("export PATH="),
        "{}",
        io.err
    );
    let g = Fixture::new();
    g.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    let (_, io) = g.yes("install ifhf");
    assert!(!io.err.contains("PATH"), "{}", io.err);
}

#[test]
fn install_dir_can_be_configured() {
    let mut f = Fixture::new();
    let custom = f.dir.path().join("custom/bin");
    f.config =
        ifhx::config::Config::parse(&format!("[install]\ndir = '{}'\n", custom.display()), "t")
            .unwrap();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    let (code, io) = f.yes("install ifhf");
    assert_eq!(code, 0, "{}", io.err);
    assert!(custom.join("ifhf").exists(), "directory is created");
}

// ---- update ---------------------------------------------------------------------------

#[test]
fn update_replaces_a_tool_with_a_newer_release() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "old");
    f.release("Disc0wd/ifhf", "ifhf", "v0.2.0", "new");
    f.release("Disc0wd/ifhx", "ifhx", "v0.1.0", "same-as-running"); // ifhx itself is current
    let mut io = TestIo::answering(&["y"]);
    assert_eq!(f.run(&mut io, "update"), 0, "{}", io.err);
    assert_eq!(
        std::fs::read_to_string(f.bin().join("ifhf")).unwrap(),
        "new"
    );
    assert!(
        io.out.contains("1 update(s) available")
            && io.out.contains("0.1.0")
            && io.out.contains("0.2.0"),
        "{}",
        io.out
    );
    assert!(io.out.contains("Updated: ifhf 0.2.0"), "{}", io.out);
    assert_eq!(f.state().tools["ifhf"].version, "v0.2.0");
    assert_eq!(
        std::fs::read_to_string(&f.self_exe).unwrap(),
        "the running ifhx",
        "ifhx untouched when current"
    );
}

#[test]
fn update_with_nothing_newer_says_so_and_asks_nothing() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "x");
    f.release("Disc0wd/ifhx", "ifhx", "v0.1.0", "x");
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "update"), 0);
    assert!(io.out.contains("Everything is up to date."), "{}", io.out);
    assert!(io.asked.is_empty());
}

#[test]
fn update_updates_ifhx_itself_last_and_replaces_the_running_binary() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "old");
    f.release("Disc0wd/ifhf", "ifhf", "v0.2.0", "new-ifhf");
    f.release("Disc0wd/ifhx", "ifhx", "v9.0.0", "new-ifhx");
    let (code, io) = f.yes("update");
    assert_eq!(code, 0, "{}", io.err);
    assert_eq!(std::fs::read_to_string(&f.self_exe).unwrap(), "new-ifhx");
    assert!(io.out.contains("ifhx itself was updated"), "{}", io.out);
    let a = io.out.find("Updated: ").unwrap();
    assert!(io.out[a..].contains("ifhf 0.2.0") && io.out[a..].contains("ifhx 9.0.0"));
    assert!(
        !f.state().tools.contains_key("ifhx"),
        "the running program is never recorded in state"
    );
    let order: Vec<String> = f
        .fetcher
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("file "))
        .collect();
    assert!(
        order[1].contains("Disc0wd/ifhf") && order[2].contains("Disc0wd/ifhx"),
        "{order:?}"
    );
}

#[test]
fn update_one_named_tool_only() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "old");
    f.release("Disc0wd/ifhf", "ifhf", "v0.2.0", "new");
    f.release("Disc0wd/ifhx", "ifhx", "v9.0.0", "new-ifhx");
    let (code, _) = f.yes("update ifhf");
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read_to_string(&f.self_exe).unwrap(),
        "the running ifhx"
    );
    let (code, io) = f.yes("update nope");
    assert_eq!(code, 3);
    assert!(io.err.contains("'nope' is not installed"), "{}", io.err);
}

#[test]
fn a_failed_update_keeps_the_working_version() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "old-working");
    f.release("Disc0wd/ifhf", "ifhf", "v0.2.0", "new");
    f.fetcher.files.lock().unwrap().insert(
        "https://github.com/Disc0wd/ifhf/releases/download/v0.2.0/ifhf-x86_64-linux".into(),
        b"corrupt".to_vec(),
    );
    let (code, io) = f.yes("update ifhf");
    assert_eq!(code, 1);
    assert!(
        io.err.contains("Could not update ifhf") && io.err.contains("checksum"),
        "{}",
        io.err
    );
    assert_eq!(
        std::fs::read_to_string(f.bin().join("ifhf")).unwrap(),
        "old-working"
    );
    assert_eq!(
        f.state().tools["ifhf"].version,
        "v0.1.0",
        "record unchanged"
    );
}

#[test]
fn partial_update_failure_reports_exactly_what_happened() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "old");
    f.release("Disc0wd/ifhf", "ifhf", "v0.2.0", "new");
    f.release("Disc0wd/ifhx", "ifhx", "v9.0.0", "x");
    f.fetcher.files.lock().unwrap().insert(
        "https://github.com/Disc0wd/ifhx/releases/download/v9.0.0/ifhx-x86_64-linux".into(),
        b"corrupt".to_vec(),
    );
    let (code, io) = f.yes("update");
    assert_eq!(code, 1);
    assert!(
        io.err.contains("Succeeded: ifhf 0.2.0") && io.err.contains("Failed: ifhx"),
        "{}",
        io.err
    );
    assert_eq!(
        std::fs::read_to_string(&f.self_exe).unwrap(),
        "the running ifhx"
    );
}

#[test]
fn a_dead_network_during_update_is_an_error_not_up_to_date() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "x");
    f.fetcher.down.lock().unwrap().push("Disc0wd".into());
    let (code, io) = f.yes("update");
    assert_eq!(code, 1, "{}", io.err);
    assert!(io.err.contains("Could not check for updates"), "{}", io.err);
    assert!(!io.out.contains("up to date"));
}

#[test]
fn one_unreachable_tool_does_not_block_the_others() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "x");
    f.release("Disc0wd/ifhx", "ifhx", "v9.0.0", "new-ifhx");
    f.fetcher.down.lock().unwrap().push("Disc0wd/ifhf".into());
    let (code, io) = f.yes("update");
    assert_eq!(code, 0, "{}", io.err);
    assert!(io.err.contains("Could not check ifhf"), "{}", io.err);
    assert_eq!(std::fs::read_to_string(&f.self_exe).unwrap(), "new-ifhx");
}

#[test]
fn updates_command_lists_status_without_changing_anything() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "x");
    f.release("Disc0wd/ifhf", "ifhf", "v0.2.0", "new");
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "updates"), 0, "{}", io.err);
    assert!(
        io.out.contains("ifhf\t0.1.0\t0.2.0\tupdate available"),
        "{}",
        io.out
    );
    assert!(
        io.out.contains("ifhx\t") && io.out.contains("no releases yet"),
        "{}",
        io.out
    );
    assert_eq!(std::fs::read_to_string(f.bin().join("ifhf")).unwrap(), "x");
}

#[test]
fn a_source_build_is_replaced_by_the_first_real_release() {
    let f = Fixture::new();
    f.yes("install ifhf --source"); // no releases: built from main
    assert_eq!(f.state().tools["ifhf"].version, "main");
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "prebuilt");
    let (code, io) = f.yes("update ifhf");
    assert_eq!(code, 0, "{}", io.err);
    assert_eq!(
        std::fs::read_to_string(f.bin().join("ifhf")).unwrap(),
        "prebuilt"
    );
    assert_eq!(f.state().tools["ifhf"].method, "release");
}

// ---- remove / list / available / info -----------------------------------------------

#[test]
fn remove_deletes_the_binary_and_the_record_only() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "x");
    let mut io = TestIo::answering(&["y"]);
    assert_eq!(f.run(&mut io, "remove ifhf"), 0, "{}", io.err);
    assert!(!f.bin().join("ifhf").exists() && f.state().tools.is_empty());
    assert!(
        io.out.contains("configuration and data are not touched")
            && io.out.contains("Removed: ifhf"),
        "{}",
        io.out
    );
    assert_eq!(
        f.run(&mut TestIo::non_interactive(), "remove ifhf --yes"),
        3,
        "second removal: not installed"
    );
}

#[test]
fn remove_tolerates_a_binary_that_is_already_gone() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "x");
    std::fs::remove_file(f.bin().join("ifhf")).unwrap();
    let (code, io) = f.yes("remove ifhf");
    assert_eq!(code, 0, "{}", io.err);
    assert!(f.state().tools.is_empty());
}

#[test]
fn ifhx_cannot_remove_itself() {
    let f = Fixture::new();
    let (code, io) = f.yes("remove ifhx");
    assert_eq!(code, 1);
    assert!(
        io.err.contains("cannot remove itself")
            && io.err.contains(&f.self_exe.display().to_string()),
        "{}",
        io.err
    );
    assert!(f.self_exe.exists());
}

#[test]
fn list_shows_installed_tools_and_ifhx_itself() {
    let f = Fixture::new();
    let mut io = TestIo::non_interactive();
    f.run(&mut io, "list");
    assert_eq!(
        io.out,
        format!(
            "ifhx\t{}\tself\t{}\n",
            env!("CARGO_PKG_VERSION"),
            f.self_exe.display()
        )
    );
    ifhf_installed(&f, "v0.1.0", "x");
    let mut io = TestIo::non_interactive();
    f.run(&mut io, "list");
    assert!(
        io.out.starts_with(&format!(
            "ifhf\t0.1.0\trelease\t{}\n",
            f.bin().join("ifhf").display()
        )),
        "{}",
        io.out
    );
}

#[test]
fn available_lists_the_registry_with_status() {
    let f = Fixture::new();
    ifhf_installed(&f, "v0.1.0", "x");
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "available"), 0);
    assert!(
        io.out
            .contains("ifhf\tA friendly command-line frontend for Flatpak\tinstalled 0.1.0"),
        "{}",
        io.out
    );
    assert!(
        io.out
            .contains("ifhx\tInstalls and updates the IFHX tool suite\tinstalled "),
        "{}",
        io.out
    );
}

#[test]
fn info_shows_details_and_latest_release() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.4.0", "x");
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "info ifhf"), 0, "{}", io.err);
    for want in [
        "ifhf",
        "Source:",
        "https://github.com/Disc0wd/ifhf",
        "Installed:",
        "no",
        "Latest:",
        "0.4.0",
    ] {
        assert!(io.out.contains(want), "{want}: {}", io.out);
    }
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "info nope"), 3);
    f.fetcher.down.lock().unwrap().push("Disc0wd/ifhf".into());
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "info ifhf"), 0);
    assert!(io.out.contains("Latest:    unknown"), "{}", io.out);
}

// ---- registry layers --------------------------------------------------------------------

#[test]
fn a_fresh_cache_means_no_registry_fetch() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    f.yes("available");
    assert_eq!(f.fetcher.fetches(REG_URL), 1);
    f.yes("available");
    f.yes("list");
    assert_eq!(
        f.fetcher.fetches(REG_URL),
        1,
        "second command reads the cache"
    );
    assert!(f.dirs.registry_cache.exists());
}

#[test]
fn sync_forces_a_refresh_and_reports_the_tool_count() {
    let f = Fixture::new();
    f.yes("available");
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "sync"), 0, "{}", io.err);
    assert!(
        io.out.contains("Tool list refreshed: 3 tools."),
        "{}",
        io.out
    );
    assert_eq!(f.fetcher.fetches(REG_URL), 2);
    // A new tool published in the registry appears without a new ifhx.
    let extended =
        format!("{BUILTIN}\n[tools.ifhz]\nrepo='Disc0wd/ifhz'\ndescription='A future tool'\n");
    f.fetcher
        .texts
        .lock()
        .unwrap()
        .insert(REG_URL.into(), extended);
    f.run(&mut TestIo::non_interactive(), "sync");
    let mut io = TestIo::non_interactive();
    f.run(&mut io, "available");
    assert!(
        io.out.contains("ifhz\tA future tool\tnot installed"),
        "{}",
        io.out
    );
}

#[test]
fn offline_with_a_cached_registry_uses_it_and_says_so() {
    let mut f = Fixture::new();
    f.yes("available"); // primes the cache
    f.config = ifhx::config::Config::parse("[registry]\nmax_age_hours = 0\n", "t").unwrap(); // force staleness
    f.fetcher.down.lock().unwrap().push(REG_URL.into());
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "available"), 0, "{}", io.err);
    assert!(
        io.err.contains("Could not refresh the tool list") && io.err.contains("using a copy from"),
        "{}",
        io.err
    );
    assert!(io.out.contains("ifhf"));
}

#[test]
fn offline_with_no_cache_falls_back_to_the_built_in_list() {
    let f = Fixture::new();
    f.fetcher.down.lock().unwrap().push(REG_URL.into());
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "available"), 0, "{}", io.err);
    assert!(
        io.err.contains("using the list built into this version"),
        "{}",
        io.err
    );
    assert!(io.out.contains("ifhf") && io.out.contains("ifhx"));
    assert!(
        !f.dirs.registry_cache.exists(),
        "a fallback is never written to the cache"
    );
}

#[test]
fn a_garbage_registry_is_not_cached_or_trusted() {
    let f = Fixture::new();
    f.fetcher
        .texts
        .lock()
        .unwrap()
        .insert(REG_URL.into(), "<html>captive portal</html>".into());
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "available"), 0);
    assert!(io.err.contains("built into this version"), "{}", io.err);
    assert!(!f.dirs.registry_cache.exists());
    f.fetcher.texts.lock().unwrap().insert(
        REG_URL.into(),
        "[tools.evil]\nrepo='../../x'\ndescription='d'\n".into(),
    );
    let mut io = TestIo::non_interactive();
    f.run(&mut io, "available");
    assert!(!io.out.contains("evil"), "{}", io.out);
}

#[test]
fn sync_fails_loudly_when_the_network_is_down() {
    let f = Fixture::new();
    f.fetcher.down.lock().unwrap().push(REG_URL.into());
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "sync"), 1);
    assert!(io.err.contains("could not be refreshed"), "{}", io.err);
}

#[test]
fn a_damaged_state_file_is_reported_not_silently_reset() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.dirs.state_file.parent().unwrap()).unwrap();
    std::fs::write(&f.dirs.state_file, "tools = [[[").unwrap();
    let mut io = TestIo::non_interactive();
    assert_eq!(f.run(&mut io, "list"), 1);
    assert!(io.err.contains("damaged"), "{}", io.err);
    assert_eq!(
        std::fs::read_to_string(&f.dirs.state_file).unwrap(),
        "tools = [[[",
        "left for the user to fix"
    );
}

// ---- presentation -----------------------------------------------------------------------

#[test]
fn terminal_output_is_styled_and_pipes_stay_plain() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    let mut io = TestIo::term(&["y"]);
    assert_eq!(f.run(&mut io, "install ifhf"), 0, "{}", io.err);
    assert!(io.out.contains("\x1b["), "{:?}", io.out);
    let p = strip(&io.out);
    assert!(
        p.contains("┃ ifhf") && p.contains("┃ Version") && p.contains("✓ Installed ifhf 0.1.0"),
        "{p}"
    );
    assert_eq!(strip(&io.asked[0]), "Install ifhf? [Y/n] ");
    let mut io = TestIo::term(&[]);
    f.run(&mut io, "list");
    assert!(strip(&io.out).starts_with("Tool"), "header on a terminal");
    let mut io = TestIo::term(&[]);
    io.color = false;
    f.run(&mut io, "list --no-color");
    assert!(!io.out.contains('\x1b'));
}

#[test]
fn errors_and_cancellation_on_a_terminal() {
    let f = Fixture::new();
    let mut io = TestIo::term(&[]);
    assert_eq!(f.run(&mut io, "install nope --yes"), 3);
    assert!(
        strip(&io.err).starts_with("✗ There is no tool called 'nope' in the registry."),
        "{}",
        io.err
    );
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    let mut io = TestIo::term(&["n"]);
    assert_eq!(f.run(&mut io, "install ifhf"), 6);
    assert_eq!(strip(&io.err), "Cancelled.\n");
}

#[test]
fn ascii_locale_gets_ascii_symbols() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "x");
    let mut io = TestIo::term(&["y"]);
    io.utf8 = false;
    f.run(&mut io, "install ifhf");
    let all = strip(&io.out);
    assert!(
        all.is_ascii() && all.contains("+ Installed ifhf") && all.contains("| ifhf"),
        "{all}"
    );
}

#[test]
fn usage_help_and_version() {
    let f = Fixture::new();
    for a in ["install", "frobnicate", "--bogus", "update --tag v1"] {
        let mut io = TestIo::non_interactive();
        assert_eq!(f.run(&mut io, a), 2, "{a}");
        assert!(io.err.contains("ifhx --help"), "{a}: {}", io.err);
    }
    for a in ["", "--help", "help"] {
        let mut io = TestIo::non_interactive();
        assert_eq!(f.run(&mut io, a), 0);
        assert!(io.out.contains("USAGE") && io.out.contains("EXIT STATUS"));
    }
    let mut io = TestIo::non_interactive();
    f.run(&mut io, "--version");
    assert_eq!(io.out, format!("ifhx {}\n", env!("CARGO_PKG_VERSION")));
    let mut io = TestIo::term(&[]);
    f.run(&mut io, "--help");
    assert!(io.out.contains("\x1b[1mUSAGE") && strip(&io.out) == ifhx::cli::HELP);
}

// ---- never overwrite what ifhx did not install ----------------------------------------

#[test]
fn an_existing_file_that_ifhx_did_not_install_is_left_alone() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "prebuilt");
    std::fs::create_dir_all(f.bin()).unwrap();
    std::fs::write(f.bin().join("ifhf"), "the user's own program").unwrap();
    let (code, io) = f.yes("install ifhf");
    assert_eq!(code, 1);
    assert!(io.err.contains("not installed by ifhx"), "{}", io.err);
    assert_eq!(
        std::fs::read_to_string(f.bin().join("ifhf")).unwrap(),
        "the user's own program"
    );
    assert!(f.state().tools.is_empty());
}

#[test]
fn an_installed_file_changed_by_the_user_is_not_overwritten_by_update() {
    let f = Fixture::new();
    f.release("Disc0wd/ifhf", "ifhf", "v0.1.0", "prebuilt");
    assert_eq!(f.yes("install ifhf").0, 0);
    std::fs::write(f.bin().join("ifhf"), "patched by hand").unwrap();
    f.release("Disc0wd/ifhf", "ifhf", "v0.2.0", "newer");
    let (code, io) = f.yes("update ifhf");
    assert_ne!(code, 0);
    assert!(
        io.err.contains("was changed after ifhx installed it"),
        "{}",
        io.err
    );
    assert_eq!(
        std::fs::read_to_string(f.bin().join("ifhf")).unwrap(),
        "patched by hand"
    );
}
