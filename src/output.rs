//! Pure formatting, the same visual language as ifhf: colour and symbols only on a terminal,
//! plain stable text when piped.

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub color: bool,
    pub fancy: bool,
    pub unicode: bool,
}

impl Style {
    pub const PLAIN: Style = Style {
        color: false,
        fancy: false,
        unicode: false,
    };

    fn paint(self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    pub fn bold(self, s: &str) -> String {
        self.paint("1", s)
    }
    pub fn dim(self, s: &str) -> String {
        self.paint("2", s)
    }
    pub fn red(self, s: &str) -> String {
        self.paint("31", s)
    }
    pub fn green(self, s: &str) -> String {
        self.paint("32", s)
    }
    pub fn yellow(self, s: &str) -> String {
        self.paint("33", s)
    }
    pub fn accent(self, s: &str) -> String {
        self.paint("94", s)
    }
    fn bold_red(self, s: &str) -> String {
        self.paint("1;31", s)
    }
    fn sym(self, uni: &'static str, ascii: &'static str) -> &'static str {
        if self.unicode { uni } else { ascii }
    }
    pub fn ok_mark(self) -> String {
        self.green(self.sym("✓", "+"))
    }
    pub fn fail_mark(self) -> String {
        self.red(self.sym("✗", "x"))
    }
    pub fn warn_mark(self) -> String {
        self.yellow("!")
    }
    pub fn step_mark(self) -> String {
        self.accent(self.sym("→", "->"))
    }
    fn bar(self) -> String {
        self.accent(self.sym("┃", "|"))
    }
    pub fn arrow(self) -> &'static str {
        self.sym("→", "->")
    }
}

pub fn table(
    header: &[&str],
    rows: &[Vec<String>],
    pretty: bool,
    style: Style,
    paint: &dyn Fn(usize, &str) -> String,
) -> String {
    let mut out = String::new();
    if !pretty {
        for r in rows {
            out.push_str(&r.join("\t"));
            out.push('\n');
        }
        return out;
    }
    let mut w: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(c.chars().count());
        }
    }
    let line = |cells: Vec<&str>, head: bool| {
        let mut s = String::new();
        for (i, c) in cells.iter().enumerate() {
            let pad = " ".repeat(w[i] - c.chars().count());
            s.push_str(&if head { style.dim(c) } else { paint(i, c) });
            if i + 1 < cells.len() {
                s.push_str(&pad);
                s.push_str("  ");
            }
        }
        s.push('\n');
        s
    };
    out.push_str(&line(header.to_vec(), true));
    for r in rows {
        out.push_str(&line(r.iter().map(String::as_str).collect(), false));
    }
    out
}

/// One block of a plan: `title` then `key: value` rows, with an accent bar on a terminal.
pub fn block(style: Style, title: &str, rows: &[(&str, String)]) -> String {
    if style.fancy {
        let bar = style.bar();
        let mut o = format!("{bar} {}\n", style.bold(title));
        for (k, v) in rows {
            o.push_str(&format!("{bar} {} {v}\n", style.dim(&format!("{k:<9}"))));
        }
        o.push('\n');
        return o;
    }
    let mut o = format!("{title}\n");
    for (k, v) in rows {
        o.push_str(&format!("  {:<11}{v}\n", format!("{k}:")));
    }
    o.push('\n');
    o
}

pub fn done_lines(style: Style, verb: &str, names: &[String]) -> String {
    if !style.fancy {
        return format!("{verb}: {}\n", names.join(", "));
    }
    names
        .iter()
        .map(|n| format!("{} {verb} {}\n", style.ok_mark(), style.bold(n)))
        .collect()
}

pub fn ok_line(style: Style, text: &str) -> String {
    if style.fancy {
        format!("{} {text}\n", style.ok_mark())
    } else {
        format!("{text}\n")
    }
}

pub fn note(style: Style, text: &str) -> String {
    if style.fancy {
        format!("{} {}\n", style.warn_mark(), style.yellow(text))
    } else {
        format!("Note: {text}\n")
    }
}

pub fn error(style: Style, msg: &str, tail: &str) -> String {
    if !style.fancy {
        return format!("ifhx: {msg}{tail}\n");
    }
    let mut lines = msg.lines();
    let first = lines.next().unwrap_or("");
    let mut out = format!("{} {}\n", style.fail_mark(), style.bold_red(first));
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    if !tail.is_empty() {
        out.push_str(&style.dim(tail.trim_start_matches('\n')));
        out.push('\n');
    }
    out
}

pub fn cancelled(style: Style) -> String {
    if style.fancy {
        format!("{}\n", style.dim("Cancelled."))
    } else {
        "ifhx: Cancelled.\n".into()
    }
}

pub fn prompt(style: Style, question: &str, default_yes: bool) -> String {
    if !style.fancy {
        return format!(
            "{question} {} ",
            if default_yes { "[Y/n]" } else { "[y/N]" }
        );
    }
    let hint = if default_yes {
        format!("[{}/n]", style.bold("Y"))
    } else {
        format!("[y/{}]", style.bold("N"))
    };
    format!("{} {hint} ", style.bold(question))
}

pub fn help(text: &str, style: Style) -> String {
    if !style.fancy {
        return text.to_string();
    }
    let mut out = String::new();
    let mut section = "";
    for line in text.lines() {
        let head = line.split_whitespace().next().unwrap_or("");
        if !line.starts_with(' ') && head.len() > 2 && head.chars().all(|c| c.is_ascii_uppercase())
        {
            section = if head == "EXIT" { "exit" } else { "" };
            out.push_str(&style.bold(line));
            out.push('\n');
            continue;
        }
        if section != "exit" && line.starts_with("  ") && !line.starts_with("   ") {
            let rest = &line[2..];
            if let Some(i) = rest.find("  ") {
                out.push_str(&format!("  {}{}\n", style.accent(&rest[..i]), &rest[i..]));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLOR: Style = Style {
        color: true,
        fancy: true,
        unicode: true,
    };
    const NOCOLOR: Style = Style {
        color: false,
        fancy: true,
        unicode: true,
    };
    const ASCII: Style = Style {
        color: false,
        fancy: true,
        unicode: false,
    };

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

    fn views(st: Style) -> Vec<String> {
        vec![
            block(
                st,
                "ifhf",
                &[("Version", "0.1.0".into()), ("Install", "/x/ifhf".into())],
            ),
            done_lines(st, "Installed", &["ifhf".into(), "ifhx".into()]),
            ok_line(st, "Fine."),
            note(st, "careful"),
            error(st, "bad\nmore", "\n\nTry: ifhx --help"),
            cancelled(st),
            prompt(st, "Install ifhf?", true),
        ]
    }

    #[test]
    fn colour_only_adds_escape_codes() {
        for (c, n) in views(COLOR).iter().zip(views(NOCOLOR)) {
            assert!(c.contains("\x1b["), "{c:?}");
            assert_eq!(strip(c), n);
        }
    }

    #[test]
    fn ascii_and_plain_modes() {
        assert!(views(ASCII).iter().all(|v| v.is_ascii()));
        let p = views(Style::PLAIN);
        assert!(p.iter().all(|v| v.is_ascii() && !v.contains('\x1b')));
        assert_eq!(p[0], "ifhf\n  Version:   0.1.0\n  Install:   /x/ifhf\n\n");
        assert_eq!(p[1], "Installed: ifhf, ifhx\n");
        assert_eq!(p[3], "Note: careful\n");
        assert_eq!(p[4], "ifhx: bad\nmore\n\nTry: ifhx --help\n");
        assert_eq!(p[6], "Install ifhf? [Y/n] ");
    }

    #[test]
    fn table_alignment_survives_colour() {
        let rows = vec![
            vec!["a".to_string(), "bbb".to_string()],
            vec!["cc".to_string(), "d".to_string()],
        ];
        let plain = table(&["X", "Y"], &rows, true, NOCOLOR, &|_, c| c.to_string());
        let col = table(&["X", "Y"], &rows, true, COLOR, &|i, c| {
            if i == 0 {
                COLOR.bold(c)
            } else {
                COLOR.accent(c)
            }
        });
        assert_eq!(strip(&col), plain);
        assert_eq!(
            table(&["X", "Y"], &rows, false, NOCOLOR, &|_, c| c.to_string()),
            "a\tbbb\ncc\td\n"
        );
    }
}
