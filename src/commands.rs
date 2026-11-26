use crate::cli::{self, Command, HELP, Invocation, Opts};
use crate::config::{Config, Dirs};
use crate::error::{Error, Result};
use crate::fetch::Fetcher;
use crate::foreign::{self, Staged};
use crate::github;
use crate::install::{self, Builder, Plan};
use crate::output::{self, Style};
use crate::registry::{self, Registry, Tool};
use crate::state::{Record, State};
use crate::verify::Verifier;
use crate::version;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Everything the commands need from the terminal. Tests script it.
pub trait Io {
    fn out(&mut self, s: &str);
    fn err(&mut self, s: &str);
    fn ask(&mut self, prompt: &str) -> Option<String>;
    fn interactive(&self) -> bool;
    fn stdout_is_tty(&self) -> bool;
    fn color_allowed(&self) -> bool;
    fn stderr_is_tty(&self) -> bool;
    fn stderr_color_allowed(&self) -> bool;
    fn unicode(&self) -> bool;
}

pub struct Env<'a> {
    pub fetcher: &'a dyn Fetcher,
    pub builder: &'a dyn Builder,
    pub verifier: &'a dyn Verifier,
    pub config: &'a Config,
    pub dirs: &'a Dirs,
    pub io: &'a mut dyn Io,
    /// e.g. `x86_64-linux`.
    pub platform: String,
    /// The running ifhx binary: where `update ifhx` writes.
    pub self_exe: PathBuf,
    /// The PATH variable, to warn when the install directory is not on it.
    pub path_var: Option<String>,
    pub registry_url: String,
}

const SELF: &str = "ifhx";

/// `ifhx` is always "installed": it is the program running. Its record is virtual and never saved.
fn self_record(env: &Env) -> Record {
    Record {
        version: env!("CARGO_PKG_VERSION").into(),
        method: "self".into(),
        path: env.self_exe.clone(),
        sha256: String::new(),
        installed_at: 0,
        ..Default::default()
    }
}

fn load_state(env: &Env) -> Result<State> {
    State::load(&env.dirs.state_file)
}

fn installed_map(env: &Env, state: &State) -> BTreeMap<String, Record> {
    let mut m = state.tools.clone();
    m.insert(SELF.into(), self_record(env));
    m
}

fn bin_dir(env: &Env) -> PathBuf {
    env.config
        .install
        .dir
        .clone()
        .unwrap_or_else(|| env.dirs.default_bin.clone())
}

fn out_style(env: &Env, no_color: bool) -> Style {
    let fancy = env.io.stdout_is_tty();
    Style {
        fancy,
        color: fancy && env.config.general.color && !no_color && env.io.color_allowed(),
        unicode: env.io.unicode(),
    }
}

fn err_style(env: &Env) -> Style {
    let fancy = env.io.stderr_is_tty();
    Style {
        fancy,
        color: fancy && env.config.general.color && env.io.stderr_color_allowed(),
        unicode: env.io.unicode(),
    }
}

fn notes(env: &mut Env, list: &[String]) {
    let st = err_style(env);
    for n in list {
        env.io.err(&output::note(st, n));
    }
}

fn load_registry(env: &mut Env, force: bool) -> Registry {
    let url = env
        .config
        .registry
        .url
        .clone()
        .unwrap_or_else(|| env.registry_url.clone());
    let loaded = registry::load(
        env.fetcher,
        &url,
        &env.dirs.registry_cache,
        env.config.registry.max_age_hours,
        force,
    );
    notes(env, &loaded.notes);
    loaded.registry
}

fn confirm(env: &mut Env, st: Style, question: &str, skip: bool) -> Result<()> {
    if skip {
        return Ok(());
    }
    if !env.io.interactive() {
        return Err(Error::NeedsConfirmation(
            "Confirmation is required but input is not a terminal.\n\nRe-run with --yes to proceed without asking.".into(),
        ));
    }
    match env
        .io
        .ask(&output::prompt(st, question, true))
        .as_deref()
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("") | Some("y") | Some("yes") => Ok(()),
        _ => Err(Error::Aborted),
    }
}

fn combine(mut problems: Vec<Error>) -> Error {
    if problems.len() == 1 {
        return problems.remove(0);
    }
    let msg = problems
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n\n");
    match problems[0] {
        Error::NotFound(_) => Error::NotFound(msg),
        _ => Error::Failed(msg),
    }
}

fn unknown_tool(name: &str) -> Error {
    Error::NotFound(format!(
        "There is no tool called '{name}' in the registry.\n\nSee what is available:\n  ifhx available"
    ))
}

fn not_installed(name: &str) -> Error {
    Error::NotFound(format!(
        "'{name}' is not installed.\n\nSee what is installed:\n  ifhx list"
    ))
}

pub fn execute(env: &mut Env, inv: Invocation) -> Result<()> {
    let Invocation { cmd, opts } = inv;
    match cmd {
        Command::Help => {
            let st = out_style(env, opts.no_color);
            env.io.out(&output::help(HELP, st));
            Ok(())
        }
        Command::Version => {
            env.io.out(&format!("ifhx {}\n", env!("CARGO_PKG_VERSION")));
            Ok(())
        }
        Command::Install(n) => install(env, &opts, &n),
        Command::Add(r) => add(env, &opts, &r),
        Command::Remove(n) => remove(env, &opts, &n),
        Command::Update(n) => update(env, &opts, &n),
        Command::Updates => updates(env, &opts),
        Command::List => list(env, &opts),
        Command::Available => available(env, &opts),
        Command::Info(n) => info(env, &opts, &n),
        Command::Sync => sync(env, &opts),
    }
}

fn describe(st: Style, p: &Plan, platform: &str) -> String {
    let how = match &p.method {
        install::Method::Release { .. } => {
            format!("prebuilt binary ({platform}), signature and checksum verified")
        }
        install::Method::Source { .. } => "built from source with cargo".to_string(),
    };
    let mut rows = vec![
        ("Version", version::display(&p.version).to_string()),
        ("Method", how),
        ("Install", p.dest.display().to_string()),
    ];
    if let Some(n) = &p.note {
        rows.push(("Note", n.clone()));
    }
    output::block(st, &p.tool.name, &rows)
}

/// Warn when a freshly installed tool would not be found by name.
fn path_warning(env: &Env, dir: &Path) -> Option<String> {
    let path = env.path_var.as_deref()?;
    if std::env::split_paths(path).any(|p| p == dir) {
        return None;
    }
    Some(format!(
        "{} is not on your PATH, so the new tool will not be found by name. Add it to your shell profile:\n  export PATH=\"{}:$PATH\"",
        dir.display(),
        dir.display()
    ))
}

fn run_plans(env: &mut Env, st: Style, plans: Vec<Plan>, verb: &str, action: &str) -> Result<()> {
    let mut state = load_state(env)?;
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    let mut self_updated = false;
    for p in &plans {
        match install::execute(p, env.fetcher, env.builder, env.verifier) {
            Ok(rec) => {
                if p.tool.name == SELF {
                    self_updated = true;
                } else {
                    state.tools.insert(p.tool.name.clone(), rec);
                    // Save after every success so a later failure cannot lose the record.
                    state.save(&env.dirs.state_file)?;
                }
                done.push(format!("{} {}", p.tool.name, version::display(&p.version)));
            }
            Err(e) => failed.push((p.tool.name.clone(), e.to_string())),
        }
    }
    if !done.is_empty() {
        env.io.out(&output::done_lines(st, verb, &done));
    }
    if self_updated {
        env.io.out(&format!(
            "{}\n",
            st.dim("ifhx itself was updated; the new version runs from the next command.")
        ));
    }
    if let Some(w) = plans
        .iter()
        .filter(|p| !failed.iter().any(|f| f.0 == p.tool.name))
        .find_map(|p| {
            let dir = p.dest.parent()?;
            path_warning(env, dir)
        })
    {
        notes(env, &[w]);
    }
    if failed.is_empty() {
        return Ok(());
    }
    let mut m = String::new();
    for (n, e) in &failed {
        m.push_str(&format!("Could not {action} {n}.\n{e}\n\n"));
    }
    let m = m.trim_end().to_string();
    if done.is_empty() {
        Err(Error::Failed(m))
    } else {
        Err(Error::Partial(format!(
            "Partly completed. Succeeded: {}. Failed: {}.\n\n{m}",
            done.join(", "),
            failed
                .iter()
                .map(|f| f.0.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

fn install(env: &mut Env, opts: &Opts, names: &[String]) -> Result<()> {
    let st = out_style(env, opts.no_color);
    let reg = load_registry(env, false);
    let state = load_state(env)?;
    let have = installed_map(env, &state);
    let (mut plans, mut problems, mut already) =
        (Vec::<Plan>::new(), Vec::new(), Vec::<String>::new());
    for name in names {
        let Some(tool) = reg.get(name) else {
            problems.push(unknown_tool(name));
            continue;
        };
        if plans.iter().any(|p| p.tool.name == *name) {
            continue;
        }
        if let Some(rec) = have.get(name) {
            already.push(format!(
                "{name} {} (run `ifhx update {name}` for a newer one)",
                version::display(&rec.version)
            ));
            continue;
        }
        let dest = bin_dir(env).join(&tool.bin);
        // Never replace a file ifhx did not put there: a registry or repository must not be able to
        // shadow a command the user already has.
        if std::fs::symlink_metadata(&dest).is_ok() {
            problems.push(already_there(&dest));
            continue;
        }
        match install::plan(
            tool,
            env.fetcher,
            &env.platform,
            dest,
            opts.tag.as_deref(),
            opts.source,
        ) {
            Ok(p) => plans.push(p),
            Err(e) => problems.push(e),
        }
    }
    if !problems.is_empty() {
        return Err(combine(problems));
    }
    for a in &already {
        env.io
            .out(&format!("{}\n", st.dim(&format!("Already installed: {a}"))));
    }
    if plans.is_empty() {
        return Ok(());
    }
    for p in &plans {
        env.io.out(&describe(st, p, &env.platform));
    }
    let q = if plans.len() == 1 {
        format!("Install {}?", plans[0].tool.name)
    } else {
        format!("Install {} tools?", plans.len())
    };
    confirm(env, st, &q, opts.yes || !env.config.general.confirm)?;
    run_plans(env, st, plans, "Installed", "install")
}

/// `owner/name`, also accepted as a github.com address.
fn repo_argument(arg: &str) -> Result<String> {
    let s = arg
        .strip_prefix("https://github.com/")
        .unwrap_or(arg)
        .trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);
    if registry::repo_ok(s) {
        Ok(s.to_string())
    } else {
        Err(Error::Usage(format!(
            "'{}' is not a GitHub repository. Use the form owner/name.",
            arg.escape_debug()
        )))
    }
}

fn arch(env: &Env) -> String {
    env.platform
        .split('-')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn foreign_disabled(env: &Env) -> Error {
    let file = env.dirs.config_file.as_ref().map_or_else(
        || "~/.config/ifhx/config.toml".to_string(),
        |p| p.display().to_string(),
    );
    Error::Failed(format!(
        "ifhx only installs IFHX tools unless you allow more.\n\nTo add programs from other GitHub repositories, put this in {file}:\n\n  [registry]\n  allow_foreign = true\n\nifhx will then check that a download is exactly the file its publisher released. It does not vet the publisher."
    ))
}

fn already_there(dest: &Path) -> Error {
    Error::Failed(format!(
        "{} already exists and was not installed by ifhx, so it was left alone.\nMove or delete it, then run this again.",
        dest.display()
    ))
}

/// Another command of this name elsewhere on the system. The program's name comes from the
/// publisher, so without this a release could ship a `sudo` or `git` that runs in place of yours.
fn shadows(env: &Env, name: &str, install_dir: &Path) -> Option<PathBuf> {
    const SYSTEM: [&str; 6] = [
        "/usr/local/sbin",
        "/usr/local/bin",
        "/usr/sbin",
        "/usr/bin",
        "/sbin",
        "/bin",
    ];
    let path = env.path_var.clone().unwrap_or_default();
    std::env::split_paths(&path)
        .chain(SYSTEM.iter().map(PathBuf::from))
        .filter(|d| d.is_absolute() && d != install_dir)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Something that cannot be verified is installed only after a person says yes. `--yes` and
/// `confirm = false` do not count, and pressing Enter means no.
fn confirm_unverified(env: &mut Env, st: Style, question: &str) -> Result<()> {
    if !env.io.interactive() {
        return Err(Error::NeedsConfirmation(
            "The publisher gave no checksum, so this download cannot be verified. ifhx only installs unverified programs after you confirm at a terminal; --yes does not apply.".into(),
        ));
    }
    match env
        .io
        .ask(&output::prompt(st, question, false))
        .as_deref()
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("y") | Some("yes") => Ok(()),
        _ => Err(Error::Aborted),
    }
}

fn describe_added(st: Style, s: &Staged, dest: &Path) -> String {
    let check = match s.checked {
        Some(how) => format!("matches {how}"),
        None => "UNVERIFIED: the publisher gave no checksum to compare with".to_string(),
    };
    output::block(
        st,
        &s.name,
        &[
            ("From", format!("github.com/{}", s.repo)),
            ("Version", version::display(&s.version).to_string()),
            ("File", s.asset.clone()),
            ("Check", check),
            (
                "Trust",
                "not an IFHX tool; ifhx does not vet its publisher".into(),
            ),
            ("Install", dest.display().to_string()),
        ],
    )
}

fn add(env: &mut Env, opts: &Opts, target: &str) -> Result<()> {
    if !env.config.registry.allow_foreign {
        return Err(foreign_disabled(env));
    }
    let repo = repo_argument(target)?;
    let st = out_style(env, opts.no_color);
    let reg = load_registry(env, false);
    let mut state = load_state(env)?;
    if let Some((n, r)) = state.tools.iter().find(|(_, r)| {
        r.repo
            .as_deref()
            .is_some_and(|x| x.eq_ignore_ascii_case(&repo))
    }) {
        env.io.out(&format!(
            "{}\n",
            st.dim(&format!(
                "Already installed: {n} {} (run `ifhx update {n}` for a newer one)",
                version::display(&r.version)
            ))
        ));
        return Ok(());
    }
    if let Some(t) = reg.iter().find(|t| t.repo.eq_ignore_ascii_case(&repo)) {
        return Err(Error::Failed(format!(
            "{repo} is the IFHX tool {}. Install it with: ifhx install {}",
            t.name, t.name
        )));
    }
    let release = github::latest_release(env.fetcher, &repo)?;
    let dir = bin_dir(env);
    // Dropping `staged` on any return below removes the downloaded file.
    let staged = foreign::prepare(env.fetcher, &repo, &release, &arch(env), None, &dir)?;
    let name = staged.name.clone();
    if name == SELF || reg.get(&name).is_some() {
        return Err(Error::Failed(format!(
            "{repo} installs a program called '{name}', which is the name of an IFHX tool, so ifhx will not add it."
        )));
    }
    if state.tools.contains_key(&name) {
        return Err(Error::Failed(format!(
            "A tool called '{name}' is already installed. Remove it first: ifhx remove {name}"
        )));
    }
    let dest = dir.join(&name);
    if std::fs::symlink_metadata(&dest).is_ok() {
        return Err(already_there(&dest));
    }
    if let Some(other) = shadows(env, &name, &dir) {
        return Err(Error::Failed(format!(
            "{repo} installs a program called '{name}', but {} already provides that command. Two of them would leave it unclear which one runs, so ifhx will not add it.",
            other.display()
        )));
    }
    env.io.out(&describe_added(st, &staged, &dest));
    let question = format!("Add {name} from {repo}?");
    if staged.checked.is_none() {
        confirm_unverified(env, st, &question)?;
    } else {
        confirm(env, st, &question, opts.yes || !env.config.general.confirm)?;
    }
    let done = format!("{name} {}", version::display(&staged.version));
    let record = foreign::commit(staged, &dest)?;
    state.tools.insert(name, record);
    state.save(&env.dirs.state_file)?;
    env.io.out(&output::done_lines(st, "Installed", &[done]));
    if let Some(w) = path_warning(env, &dir) {
        notes(env, &[w]);
    }
    Ok(())
}

/// Update one tool that was added with `ifhx add`, with the same checks as when it was added.
fn update_added(env: &mut Env, st: Style, c: &Check, rec: &Record) -> Result<String> {
    let repo = &c.tool.repo;
    let release = github::latest_release(env.fetcher, repo)?;
    if !version::is_newer(&release.tag, &rec.version) {
        return Err(Error::Failed(format!(
            "GitHub reports {} as the latest release, which is not newer than the installed one.",
            release.tag
        )));
    }
    let dir = c.dest.parent().unwrap_or_else(|| Path::new("."));
    let staged = foreign::prepare(
        env.fetcher,
        repo,
        &release,
        &arch(env),
        rec.pattern.as_deref(),
        dir,
    )?;
    if staged.name != c.name {
        return Err(Error::Failed(format!(
            "The new release installs a program called '{}' instead of '{}'. Remove it and add it again if that is expected.",
            staged.name, c.name
        )));
    }
    // A publisher that gave checksums and suddenly stops is exactly what a tampered release looks like.
    if rec.verified && staged.checked.is_none() {
        return Err(Error::Verify(format!(
            "{} was verified when it was added, but this release publishes no checksum, so it was not installed.",
            c.name
        )));
    }
    if staged.checked.is_none() {
        env.io.out(&describe_added(st, &staged, &c.dest));
        confirm_unverified(
            env,
            st,
            &format!("Update {} to this unverified download?", c.name),
        )?;
    }
    let done = format!("{} {}", c.name, version::display(&staged.version));
    let record = foreign::commit(staged, &c.dest)?;
    let mut state = load_state(env)?;
    state.tools.insert(c.name.clone(), record);
    state.save(&env.dirs.state_file)?;
    Ok(done)
}

struct Check {
    name: String,
    current: String,
    /// Err = the check itself failed; Ok(None) = no releases published.
    latest: Result<Option<String>>,
    tool: Tool,
    dest: PathBuf,
    /// SHA-256 recorded at install time; empty for ifhx itself.
    sha256: String,
    /// The install record of a tool added with `ifhx add`; None for registry tools.
    added: Option<Record>,
}

impl Check {
    fn newer(&self) -> Option<&str> {
        match &self.latest {
            Ok(Some(t)) if version::is_newer(t, &self.current) => Some(t),
            _ => None,
        }
    }
}

fn check_all(env: &mut Env, names: &[String]) -> Result<Vec<Check>> {
    let reg = load_registry(env, false);
    let state = load_state(env)?;
    let have = installed_map(env, &state);
    let mut wanted: Vec<String> = if names.is_empty() {
        have.keys().cloned().collect()
    } else {
        names.to_vec()
    };
    // ifhx last: replacing it should be the final act.
    wanted.sort_by_key(|n| n == SELF);
    wanted.dedup();
    let mut problems = Vec::new();
    for n in &wanted {
        if !have.contains_key(n) {
            problems.push(not_installed(n));
        }
    }
    if !problems.is_empty() {
        return Err(combine(problems));
    }
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for n in wanted {
        let rec = &have[&n];
        let tool = match &rec.repo {
            Some(_) if !env.config.registry.allow_foreign => {
                skipped.push(format!(
                    "{n} was added from outside the IFHX suite and allow_foreign is off, so it is not checked."
                ));
                continue;
            }
            Some(repo) => Tool::added(&n, repo),
            None => match reg.get(&n) {
                Some(t) => t.clone(),
                None => {
                    skipped.push(format!(
                        "{n} is installed but not in the registry, so it cannot be updated."
                    ));
                    continue;
                }
            },
        };
        out.push(Check {
            latest: env.fetcher.latest_tag(&tool.repo),
            name: n.clone(),
            current: rec.version.clone(),
            tool,
            dest: rec.path.clone(),
            sha256: rec.sha256.clone(),
            added: rec.repo.is_some().then(|| rec.clone()),
        });
    }
    notes(env, &skipped);
    Ok(out)
}

fn failed_checks(env: &mut Env, checks: &[Check]) -> Result<()> {
    let errs: Vec<String> = checks
        .iter()
        .filter_map(|c| {
            c.latest.as_ref().err().map(|e| {
                format!(
                    "Could not check {}: {}",
                    c.name,
                    e.to_string().lines().next().unwrap_or("")
                )
            })
        })
        .collect();
    notes(env, &errs);
    if !checks.is_empty() && errs.len() == checks.len() {
        return Err(Error::Network(
            "Could not check for updates. Check your network connection.".into(),
        ));
    }
    Ok(())
}

fn updates(env: &mut Env, opts: &Opts) -> Result<()> {
    let st = out_style(env, opts.no_color);
    let checks = check_all(env, &[])?;
    failed_checks(env, &checks)?;
    let rows: Vec<Vec<String>> = checks
        .iter()
        .map(|c| {
            let (latest, status) = match &c.latest {
                Err(_) => ("?".to_string(), "check failed"),
                Ok(None) => ("-".to_string(), "no releases yet"),
                Ok(Some(t)) if c.newer().is_some() => {
                    (version::display(t).to_string(), "update available")
                }
                Ok(Some(t)) => (version::display(t).to_string(), "up to date"),
            };
            vec![
                c.name.clone(),
                version::display(&c.current).to_string(),
                latest,
                status.to_string(),
            ]
        })
        .collect();
    let pretty = env.io.stdout_is_tty();
    env.io.out(&output::table(
        &["Tool", "Installed", "Latest", "Status"],
        &rows,
        pretty,
        st,
        &|col, c| match (col, c) {
            (0, _) => st.bold(c),
            (3, "update available") => st.yellow(c),
            (3, "up to date") => st.green(c),
            (3, _) => st.dim(c),
            _ => st.dim(c),
        },
    ));
    Ok(())
}

fn update(env: &mut Env, opts: &Opts, names: &[String]) -> Result<()> {
    let st = out_style(env, opts.no_color);
    let checks = check_all(env, names)?;
    failed_checks(env, &checks)?;
    let todo: Vec<&Check> = checks.iter().filter(|c| c.newer().is_some()).collect();
    if todo.is_empty() {
        env.io
            .out(&output::ok_line(st, "Everything is up to date."));
        return Ok(());
    }
    let rows: Vec<Vec<String>> = todo
        .iter()
        .map(|c| {
            vec![
                c.name.clone(),
                version::display(&c.current).to_string(),
                st.arrow().to_string(),
                version::display(c.newer().unwrap_or("")).to_string(),
            ]
        })
        .collect();
    env.io.out(&format!(
        "{}\n\n",
        st.bold(&format!("{} update(s) available:", todo.len()))
    ));
    let pretty = env.io.stdout_is_tty();
    env.io.out(&output::table(
        &["Tool", "Installed", "", "Latest"],
        &rows,
        pretty,
        st,
        &|col, c| match col {
            0 => st.bold(c),
            3 => st.green(c),
            _ => st.dim(c),
        },
    ));
    env.io.out("\n");
    confirm(
        env,
        st,
        "Apply these updates?",
        opts.yes || !env.config.general.confirm,
    )?;
    let mut plans = Vec::new();
    let mut added = Vec::new();
    let mut failed = Vec::new();
    for c in todo {
        // The installed file must still be the one ifhx wrote; otherwise the user changed it.
        if !c.sha256.is_empty() && install::sha256_file(&c.dest).is_ok_and(|now| now != c.sha256) {
            failed.push((
                c.name.clone(),
                format!(
                    "{} was changed after ifhx installed it, so it was left alone.\nRemove it with `ifhx remove {}` and install it again.",
                    c.dest.display(),
                    c.name
                ),
            ));
            continue;
        }
        if let Some(rec) = &c.added {
            if opts.source {
                failed.push((
                    c.name.clone(),
                    "Tools added with `ifhx add` are never built from source.".to_string(),
                ));
            } else {
                added.push((c, rec));
            }
            continue;
        }
        match install::plan(
            &c.tool,
            env.fetcher,
            &env.platform,
            c.dest.clone(),
            c.newer(),
            opts.source,
        ) {
            Ok(p) => plans.push(p),
            Err(e) => failed.push((c.name.clone(), e.to_string())),
        }
    }
    let result = if plans.is_empty() {
        Ok(())
    } else {
        run_plans(env, st, plans, "Updated", "update")
    };
    let mut updated = Vec::new();
    for (c, rec) in added {
        match update_added(env, st, c, rec) {
            Ok(done) => updated.push(done),
            Err(e) => failed.push((c.name.clone(), e.to_string())),
        }
    }
    if !updated.is_empty() {
        env.io.out(&output::done_lines(st, "Updated", &updated));
    }
    if failed.is_empty() {
        return result;
    }
    let mut m = String::new();
    for (n, e) in &failed {
        m.push_str(&format!("Could not update {n}.\n{e}\n\n"));
    }
    let m = m.trim_end().to_string();
    match result {
        Ok(()) => Err(Error::Failed(m)),
        Err(e) => Err(Error::Partial(format!("{e}\n\n{m}"))),
    }
}

fn list(env: &mut Env, opts: &Opts) -> Result<()> {
    let st = out_style(env, opts.no_color);
    let state = load_state(env)?;
    let rows: Vec<Vec<String>> = installed_map(env, &state)
        .into_iter()
        .map(|(n, r)| {
            vec![
                n,
                version::display(&r.version).to_string(),
                r.method,
                r.path.display().to_string(),
            ]
        })
        .collect();
    let pretty = env.io.stdout_is_tty();
    env.io.out(&output::table(
        &["Tool", "Version", "Source", "Path"],
        &rows,
        pretty,
        st,
        &|col, c| match col {
            0 => st.bold(c),
            _ => st.dim(c),
        },
    ));
    Ok(())
}

fn available(env: &mut Env, opts: &Opts) -> Result<()> {
    let st = out_style(env, opts.no_color);
    let reg = load_registry(env, false);
    let state = load_state(env)?;
    let have = installed_map(env, &state);
    let rows: Vec<Vec<String>> = reg
        .iter()
        .map(|t| {
            let status = have
                .get(&t.name)
                .map(|r| format!("installed {}", version::display(&r.version)))
                .unwrap_or_else(|| "not installed".into());
            vec![t.name.clone(), t.description.clone(), status]
        })
        .collect();
    let pretty = env.io.stdout_is_tty();
    env.io.out(&output::table(
        &["Tool", "Description", "Status"],
        &rows,
        pretty,
        st,
        &|col, c| match (col, c.starts_with("installed")) {
            (0, _) => st.bold(c),
            (2, true) => st.green(c),
            (2, false) => st.dim(c),
            _ => c.to_string(),
        },
    ));
    Ok(())
}

fn info(env: &mut Env, opts: &Opts, names: &[String]) -> Result<()> {
    let st = out_style(env, opts.no_color);
    let reg = load_registry(env, false);
    let state = load_state(env)?;
    let have = installed_map(env, &state);
    let mut problems = Vec::new();
    let mut blocks = Vec::new();
    for n in names {
        let own = have.get(n).and_then(|r| r.repo.as_deref());
        let Some(t) = own
            .map(|repo| Tool::added(n, repo))
            .or_else(|| reg.get(n).cloned())
        else {
            problems.push(unknown_tool(n));
            continue;
        };
        let installed = match have.get(n) {
            Some(r) => format!(
                "{} ({}) at {}",
                version::display(&r.version),
                r.method,
                r.path.display()
            ),
            None => "no".to_string(),
        };
        let latest = match env.fetcher.latest_tag(&t.repo) {
            Ok(Some(tag)) => version::display(&tag).to_string(),
            Ok(None) => "no releases yet".into(),
            Err(e) => format!("unknown ({})", e.to_string().lines().next().unwrap_or("")),
        };
        blocks.push(output::block(
            st,
            &t.name,
            &[
                ("About", t.description.clone()),
                ("Source", format!("https://github.com/{}", t.repo)),
                ("Installed", installed),
                ("Latest", latest),
            ],
        ));
    }
    env.io.out(&blocks.concat());
    if problems.is_empty() {
        Ok(())
    } else {
        Err(combine(problems))
    }
}

fn remove(env: &mut Env, opts: &Opts, names: &[String]) -> Result<()> {
    let st = out_style(env, opts.no_color);
    let mut state = load_state(env)?;
    let mut problems = Vec::new();
    let mut targets: Vec<(String, Record)> = Vec::new();
    for n in names {
        if n == SELF {
            problems.push(Error::Failed(format!(
                "ifhx cannot remove itself.\n\nTo uninstall it, delete {}",
                env.self_exe.display()
            )));
        } else if let Some(r) = state.tools.get(n) {
            if !targets.iter().any(|t| t.0 == *n) {
                targets.push((n.clone(), r.clone()));
            }
        } else {
            problems.push(not_installed(n));
        }
    }
    if !problems.is_empty() {
        return Err(combine(problems));
    }
    let blocks: String = targets
        .iter()
        .map(|(n, r)| {
            output::block(
                st,
                n,
                &[
                    ("Version", version::display(&r.version).to_string()),
                    ("Delete", r.path.display().to_string()),
                ],
            )
        })
        .collect();
    env.io.out(&blocks);
    env.io.out(&format!(
        "{}\n\n",
        st.dim("The tool's own configuration and data are not touched.")
    ));
    let q = if targets.len() == 1 {
        format!("Remove {}?", targets[0].0)
    } else {
        format!("Remove {} tools?", targets.len())
    };
    confirm(env, st, &q, opts.yes || !env.config.general.confirm)?;
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for (n, r) in targets {
        match std::fs::remove_file(&r.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // already gone: just forget it
            Err(e) => {
                failed.push((n, format!("Cannot delete {}: {e}", r.path.display())));
                continue;
            }
        }
        state.tools.remove(&n);
        state.save(&env.dirs.state_file)?;
        done.push(n);
    }
    if !done.is_empty() {
        env.io.out(&output::done_lines(st, "Removed", &done));
    }
    if failed.is_empty() {
        return Ok(());
    }
    let m = failed
        .iter()
        .map(|(n, e)| format!("Could not remove {n}.\n{e}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    Err(if done.is_empty() {
        Error::Failed(m)
    } else {
        Error::Partial(m)
    })
}

fn sync(env: &mut Env, opts: &Opts) -> Result<()> {
    let st = out_style(env, opts.no_color);
    if st.fancy {
        env.io
            .out(&format!("{} Refreshing the tool list…\n", st.step_mark()));
    }
    let url = env
        .config
        .registry
        .url
        .clone()
        .unwrap_or_else(|| env.registry_url.clone());
    let loaded = registry::load(
        env.fetcher,
        &url,
        &env.dirs.registry_cache,
        env.config.registry.max_age_hours,
        true,
    );
    if !loaded.notes.is_empty() {
        // A forced refresh that fell back is a failure the user asked to know about.
        notes(env, &loaded.notes);
        return Err(Error::Network(
            "The tool list could not be refreshed.".into(),
        ));
    }
    env.io.out(&output::ok_line(
        st,
        &format!("Tool list refreshed: {} tools.", loaded.registry.len()),
    ));
    Ok(())
}

/// Parse and execute; returns the exit code. All user-facing errors are printed here.
pub fn run(args: &[String], env: &mut Env) -> i32 {
    match cli::parse(args).and_then(|inv| execute(env, inv)) {
        Ok(()) => 0,
        Err(e) => {
            let st = err_style(env);
            if matches!(e, Error::Aborted) {
                env.io.err(&output::cancelled(st));
            } else {
                let tail = if matches!(e, Error::Usage(_)) {
                    "\n\nTry: ifhx --help"
                } else {
                    ""
                };
                env.io.err(&output::error(st, &e.to_string(), tail));
            }
            e.exit_code()
        }
    }
}
