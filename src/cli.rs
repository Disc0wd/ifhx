use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Install(Vec<String>),
    Remove(Vec<String>),
    Update(Vec<String>),
    Updates,
    List,
    Available,
    Info(Vec<String>),
    Sync,
    Help,
    Version,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Opts {
    pub yes: bool,
    /// Build from source even if a prebuilt binary exists.
    pub source: bool,
    /// Install this release tag instead of the latest.
    pub tag: Option<String>,
    pub no_color: bool,
}

pub struct Invocation {
    pub cmd: Command,
    pub opts: Opts,
}

fn usage<T>(m: impl Into<String>) -> Result<T> {
    Err(Error::Usage(m.into()))
}

pub fn parse(args: &[String]) -> Result<Invocation> {
    let mut opts = Opts::default();
    let (mut verb, mut targets): (Option<String>, Vec<String>) = (None, vec![]);
    let (mut help, mut version, mut only_targets) = (false, false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if only_targets || !a.starts_with('-') || a == "-" {
            if verb.is_none() {
                verb = Some(a.clone())
            } else {
                targets.push(a.clone())
            }
        } else if a == "--" {
            only_targets = true;
        } else if let Some(long) = a.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            match name {
                "tag" => match inline.or_else(|| it.next().cloned()) {
                    Some(v) => opts.tag = Some(v),
                    None => return usage("--tag needs a release tag, for example --tag v0.1.0."),
                },
                "source" => opts.source = true,
                "yes" | "noconfirm" => opts.yes = true,
                "no-color" => opts.no_color = true,
                "help" => help = true,
                "version" => version = true,
                _ => return usage(format!("Unknown option '--{name}'.")),
            }
        } else {
            for c in a[1..].chars() {
                match c {
                    'y' => opts.yes = true,
                    'h' => help = true,
                    'V' => version = true,
                    _ => return usage(format!("Unknown option '-{c}'.")),
                }
            }
        }
    }
    if help {
        return Ok(Invocation {
            cmd: Command::Help,
            opts,
        });
    }
    if version {
        return Ok(Invocation {
            cmd: Command::Version,
            opts,
        });
    }
    let need = |name: &str, t: Vec<String>| {
        if t.is_empty() {
            usage(format!(
                "'{name}' needs at least one tool name. See: ifhx available"
            ))
        } else {
            Ok(t)
        }
    };
    let none = |name: &str, t: &[String]| {
        if t.is_empty() {
            Ok(())
        } else {
            usage(format!(
                "'{name}' does not take arguments (got '{}').",
                t[0]
            ))
        }
    };
    let cmd = match verb.as_deref() {
        None | Some("help") => Command::Help,
        Some("version") => Command::Version,
        Some("install") => Command::Install(need("install", targets)?),
        Some("remove") => Command::Remove(need("remove", targets)?),
        Some("info") => Command::Info(need("info", targets)?),
        Some("update") => Command::Update(targets),
        Some(v @ ("updates" | "list" | "available" | "sync")) => {
            none(v, &targets)?;
            match v {
                "updates" => Command::Updates,
                "list" => Command::List,
                "available" => Command::Available,
                _ => Command::Sync,
            }
        }
        Some(other) => return usage(format!("Unknown command '{other}'. Try: ifhx --help")),
    };
    if opts.tag.is_some() && !matches!(&cmd, Command::Install(t) if t.len() == 1) {
        return usage("--tag works with 'install' and exactly one tool.");
    }
    if opts.source && !matches!(cmd, Command::Install(_) | Command::Update(_)) {
        return usage("--source only applies to 'install' and 'update'.");
    }
    Ok(Invocation { cmd, opts })
}

pub const HELP: &str = "\
ifhx - installs and updates the IFHX tool suite

USAGE
  ifhx <command> [options] [tools...]

COMMANDS
  install <tool>...   Install tools (prebuilt binary, or built from source)
  remove <tool>...    Remove tools (their own config and data are kept)
  update [<tool>...]  Update installed tools, and ifhx itself (default: all)
  updates             Show which tools have a newer release
  list                Show installed tools
  available           Show every tool in the registry
  info <tool>...      Details about a tool
  sync                Refresh the tool registry

OPTIONS
  --tag <tag>         With install: this release instead of the latest
  --source            Build from source instead of downloading a binary
  --yes, --noconfirm  Do not ask for confirmation (required when not on a terminal)
  --no-color          Disable colour (NO_COLOR is also respected)
  -h, --help          Show this help
  -V, --version       Show the version

EXIT STATUS
  0 success   1 failure   2 usage error   3 not found
  5 confirmation required (use --yes)   6 cancelled
";

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &str) -> Result<Invocation> {
        parse(&a.split_whitespace().map(String::from).collect::<Vec<_>>())
    }
    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn commands_parse() {
        assert_eq!(
            p("install ifhf ifhx").unwrap().cmd,
            Command::Install(v(&["ifhf", "ifhx"]))
        );
        assert_eq!(p("update").unwrap().cmd, Command::Update(vec![]));
        assert_eq!(p("update ifhf").unwrap().cmd, Command::Update(v(&["ifhf"])));
        assert_eq!(p("list").unwrap().cmd, Command::List);
        assert_eq!(p("").unwrap().cmd, Command::Help);
        assert_eq!(p("-V").unwrap().cmd, Command::Version);
        assert_eq!(p("install x -h").unwrap().cmd, Command::Help);
    }

    #[test]
    fn options() {
        let i = p("install ifhf --tag v0.1.0 --source -y").unwrap();
        assert_eq!(
            i.opts,
            Opts {
                yes: true,
                source: true,
                tag: Some("v0.1.0".into()),
                no_color: false
            }
        );
        assert_eq!(
            p("install ifhf --tag=v1").unwrap().opts.tag.as_deref(),
            Some("v1")
        );
        assert!(p("update --noconfirm --no-color").unwrap().opts.no_color);
    }

    #[test]
    fn usage_errors() {
        for bad in [
            "install",
            "remove",
            "info",
            "frobnicate",
            "list extra",
            "--bogus",
            "-x",
            "install ifhf --tag",
            "install a b --tag v1",
            "update --tag v1",
            "list --source",
            "sync --source",
        ] {
            assert_eq!(p(bad).err().map(|e| e.exit_code()), Some(2), "{bad}");
        }
    }
}
