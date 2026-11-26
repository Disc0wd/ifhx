#![allow(dead_code)]
use ifhx::commands::{self, Env, Io};
use ifhx::config::{Config, Dirs};
use ifhx::error::{Error, Result};
use ifhx::fetch::Fetcher;
use ifhx::install::{Builder, SourceJob};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct TestIo {
    pub out: String,
    pub err: String,
    pub answers: VecDeque<String>,
    pub asked: Vec<String>,
    pub interactive: bool,
    pub tty: bool,
    pub color: bool,
    pub err_tty: bool,
    pub utf8: bool,
}

impl TestIo {
    pub fn non_interactive() -> Self {
        TestIo {
            out: String::new(),
            err: String::new(),
            answers: VecDeque::new(),
            asked: vec![],
            interactive: false,
            tty: false,
            color: false,
            err_tty: false,
            utf8: true,
        }
    }
    pub fn answering(a: &[&str]) -> Self {
        TestIo {
            interactive: true,
            answers: a.iter().map(|s| s.to_string()).collect(),
            ..Self::non_interactive()
        }
    }
    pub fn term(a: &[&str]) -> Self {
        TestIo {
            tty: true,
            err_tty: true,
            color: true,
            ..Self::answering(a)
        }
    }
}

impl Io for TestIo {
    fn out(&mut self, s: &str) {
        self.out.push_str(s);
    }
    fn err(&mut self, s: &str) {
        self.err.push_str(s);
    }
    fn ask(&mut self, p: &str) -> Option<String> {
        self.asked.push(p.to_string());
        self.answers.pop_front()
    }
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn stdout_is_tty(&self) -> bool {
        self.tty
    }
    fn color_allowed(&self) -> bool {
        self.tty && self.color
    }
    fn stderr_is_tty(&self) -> bool {
        self.err_tty
    }
    fn stderr_color_allowed(&self) -> bool {
        self.err_tty && self.color
    }
    fn unicode(&self) -> bool {
        self.utf8
    }
}

#[derive(Default)]
pub struct MockFetcher {
    pub texts: Mutex<HashMap<String, String>>,
    pub files: Mutex<HashMap<String, Vec<u8>>>,
    /// repo -> latest tag (absent = no releases)
    pub tags: Mutex<HashMap<String, String>>,
    /// Any URL or repo containing one of these fails with a network error.
    pub down: Mutex<Vec<String>>,
    pub calls: Mutex<Vec<String>>,
}

impl MockFetcher {
    fn is_down(&self, what: &str) -> bool {
        self.down
            .lock()
            .unwrap()
            .iter()
            .any(|d| what.contains(d.as_str()))
    }
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    pub fn fetches(&self, needle: &str) -> usize {
        self.calls().iter().filter(|c| c.contains(needle)).count()
    }
}

impl Fetcher for MockFetcher {
    fn text(&self, url: &str) -> Result<String> {
        self.calls.lock().unwrap().push(format!("text {url}"));
        if self.is_down(url) {
            return Err(Error::Network(format!(
                "Could not reach {url}.\nmock: network down"
            )));
        }
        self.texts
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("Not found: {url}")))
    }
    fn file(&self, url: &str, dest: &Path) -> Result<()> {
        self.calls.lock().unwrap().push(format!("file {url}"));
        if self.is_down(url) {
            return Err(Error::Network(format!(
                "Could not reach {url}.\nmock: network down"
            )));
        }
        let bytes = self
            .files
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("Not found: {url}")))?;
        std::fs::write(dest, bytes)?;
        Ok(())
    }
    fn latest_tag(&self, repo: &str) -> Result<Option<String>> {
        self.calls.lock().unwrap().push(format!("tag {repo}"));
        if self.is_down(repo) {
            return Err(Error::Network(
                "Could not reach github.com.\nmock: network down".into(),
            ));
        }
        Ok(self.tags.lock().unwrap().get(repo).cloned())
    }
}

#[derive(Default)]
pub struct MockBuilder {
    pub jobs: Mutex<Vec<String>>,
    pub fail: Mutex<bool>,
}

impl Builder for MockBuilder {
    fn build(&self, job: &SourceJob, out: &Path) -> Result<()> {
        self.jobs
            .lock()
            .unwrap()
            .push(format!("{} {}", job.repo, job.tag.unwrap_or("HEAD")));
        if *self.fail.lock().unwrap() {
            return Err(Error::Failed(
                "Building from source needs both git and cargo (mock).".into(),
            ));
        }
        std::fs::write(
            out,
            format!("built-from-source:{}", job.tag.unwrap_or("HEAD")),
        )?;
        Ok(())
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub const REG_URL: &str = "https://reg.example/registry.toml";

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub fetcher: MockFetcher,
    pub builder: MockBuilder,
    pub config: Config,
    pub dirs: Dirs,
    pub self_exe: PathBuf,
    pub path_var: Option<String>,
}

impl Fixture {
    pub fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let bin = p.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let self_exe = bin.join("ifhx");
        std::fs::write(&self_exe, "the running ifhx").unwrap();
        let dirs = Dirs {
            config_file: None,
            state_file: p.join("state/installed.toml"),
            registry_cache: p.join("cache/registry.toml"),
            default_bin: bin.clone(),
        };
        let fetcher = MockFetcher::default();
        fetcher
            .texts
            .lock()
            .unwrap()
            .insert(REG_URL.into(), ifhx::registry::BUILTIN.into());
        Fixture {
            dir,
            fetcher,
            builder: MockBuilder::default(),
            config: Config::default(),
            dirs,
            self_exe,
            path_var: Some(bin.display().to_string()),
        }
    }

    pub fn bin(&self) -> PathBuf {
        self.dir.path().join("bin")
    }

    /// Publish a release of `repo` with a binary asset and its checksum file.
    pub fn release(&self, repo: &str, bin: &str, tag: &str, content: &str) {
        let base = format!("https://github.com/{repo}/releases/download/{tag}");
        let asset = format!("{bin}-x86_64-linux");
        self.fetcher
            .tags
            .lock()
            .unwrap()
            .insert(repo.into(), tag.into());
        self.fetcher.texts.lock().unwrap().insert(
            format!("{base}/{asset}.sha256"),
            format!("{}  {asset}\n", sha256(content.as_bytes())),
        );
        self.fetcher
            .files
            .lock()
            .unwrap()
            .insert(format!("{base}/{asset}"), content.as_bytes().to_vec());
    }

    pub fn run(&self, io: &mut TestIo, args: &str) -> i32 {
        let args: Vec<String> = args.split_whitespace().map(String::from).collect();
        let mut env = Env {
            fetcher: &self.fetcher,
            builder: &self.builder,
            config: &self.config,
            dirs: &self.dirs,
            io,
            platform: "x86_64-linux".into(),
            self_exe: self.self_exe.clone(),
            path_var: self.path_var.clone(),
            registry_url: REG_URL.into(),
        };
        commands::run(&args, &mut env)
    }

    /// Run non-interactively with --yes.
    pub fn yes(&self, args: &str) -> (i32, TestIo) {
        let mut io = TestIo::non_interactive();
        let code = self.run(&mut io, &format!("{args} --yes"));
        (code, io)
    }

    pub fn state(&self) -> ifhx::state::State {
        ifhx::state::State::load(&self.dirs.state_file).unwrap()
    }
}
