use crate::cli::{self, Command, HELP, Invocation, Opts};
use crate::config::{Config, Dirs};
use crate::error::{Error, Result};
use crate::fetch::Fetcher;
use crate::install::{self, Builder, Plan};
use crate::output::{self, Style};
use crate::registry::{self, Registry, Tool};
use crate::state::{Record, State};
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
            format!("prebuilt binary ({platform}), checksum verified")
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
        match install::execute(p, env.fetcher, env.builder) {
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

struct Check {
    name: String,
    current: String,
    /// Err = the check itself failed; Ok(None) = no releases published.
    latest: Result<Option<String>>,
    tool: Tool,
    dest: PathBuf,
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
        let Some(tool) = reg.get(&n) else {
            skipped.push(format!(
                "{n} is installed but not in the registry, so it cannot be updated."
            ));
            continue;
        };
        out.push(Check {
            latest: env.fetcher.latest_tag(&tool.repo),
            name: n.clone(),
            current: rec.version.clone(),
            tool: tool.clone(),
            dest: rec.path.clone(),
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
    let mut failed = Vec::new();
    for c in todo {
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
        let Some(t) = reg.get(n) else {
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
