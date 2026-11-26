use ifhx::commands::{self, Env, Io};
use ifhx::config::{Config, Dirs};
use ifhx::fetch::Curl;
use ifhx::install::{self, CargoBuilder};
use ifhx::registry::DEFAULT_URL;
use ifhx::verify::Minisign;
use std::io::{BufRead, IsTerminal, Write};

struct StdIo;

fn no_color_env() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

impl Io for StdIo {
    fn out(&mut self, s: &str) {
        if std::io::stdout().lock().write_all(s.as_bytes()).is_err() {
            std::process::exit(0); // closed pipe
        }
    }
    fn err(&mut self, s: &str) {
        let _ = std::io::stderr().write_all(s.as_bytes());
    }
    fn ask(&mut self, prompt: &str) -> Option<String> {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(prompt.as_bytes());
        let _ = o.flush();
        drop(o);
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim().to_string()),
        }
    }
    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal()
    }
    fn stdout_is_tty(&self) -> bool {
        std::io::stdout().is_terminal()
    }
    fn color_allowed(&self) -> bool {
        self.stdout_is_tty() && !no_color_env()
    }
    fn stderr_is_tty(&self) -> bool {
        std::io::stderr().is_terminal()
    }
    fn stderr_color_allowed(&self) -> bool {
        self.stderr_is_tty() && !no_color_env()
    }
    fn unicode(&self) -> bool {
        ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .is_some_and(|v| {
                let v = v.to_ascii_lowercase();
                v.contains("utf-8") || v.contains("utf8")
            })
    }
}

fn setup() -> ifhx::error::Result<(Config, Dirs, String, std::path::PathBuf)> {
    let dirs = Dirs::from_env()?;
    let config = Config::load(dirs.config_file.as_deref())?;
    let platform = install::platform()?;
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| {
            ifhx::error::Error::Failed(format!("Cannot locate the running ifhx binary: {e}"))
        })?;
    Ok((config, dirs, platform, exe))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (config, dirs, platform, self_exe) = match setup() {
        Ok(v) => v,
        Err(e) => {
            // --help and --version should work even on an unsupported system.
            if args
                .iter()
                .any(|a| a == "--help" || a == "-h" || a == "-V" || a == "--version")
            {
                if args.iter().any(|a| a == "-V" || a == "--version") {
                    println!("ifhx {}", env!("CARGO_PKG_VERSION"));
                } else {
                    print!("{}", ifhx::cli::HELP);
                }
                return;
            }
            eprintln!("ifhx: {e}");
            std::process::exit(e.exit_code());
        }
    };
    let mut io = StdIo;
    let mut env = Env {
        fetcher: &Curl,
        builder: &CargoBuilder,
        verifier: &Minisign,
        config: &config,
        dirs: &dirs,
        io: &mut io,
        platform,
        self_exe,
        path_var: std::env::var("PATH").ok(),
        registry_url: std::env::var("IFHX_REGISTRY").unwrap_or_else(|_| DEFAULT_URL.to_string()),
    };
    std::process::exit(commands::run(&args, &mut env));
}
