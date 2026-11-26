#!/usr/bin/env python3
"""Build docs/index.html (GitHub Pages) from site/template.html, site/style.css and registry.toml.

The tools list comes straight from registry.toml, so the page lists exactly what `ifhx install` can
install. The terminal transcript is real output saved in site/transcript.txt.
"""
import html, os, tomllib

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
site = os.path.join(root, "site")
repo = "https://github.com/Disc0wd/ifhx"

reg = tomllib.load(open(os.path.join(root, "registry.toml"), "rb"))["tools"]
order = ["ifhf", "ifhai", "ifhx"]
names = [n for n in order if n in reg] + sorted(n for n in reg if n not in order)
assert {"ifhf", "ifhai", "ifhx"} <= set(names), "the registry must list all three IFHX tools"

rows = []
for n in names:
    t = reg[n]
    r = t["repo"]
    me = " (this program)" if n == "ifhx" else ""
    rows.append(
        f"""      <li class="tool"><span class="n">{html.escape(n)}</span><p>{html.escape(t['description'])}{me}</p><span class="cmd">ifhx install {html.escape(n)}</span>
        <span class="links"><a href="https://disc0wd.github.io/{html.escape(n)}/">Page</a><a href="https://github.com/{html.escape(r)}">Source</a><a href="https://github.com/{html.escape(r)}/releases/latest">Latest release</a></span></li>"""
    )

def colour(line: str) -> str:
    esc = html.escape(line)
    if esc.startswith("$ "):
        return f'<span class="c">$</span> {esc[2:]}'
    if esc.startswith("Installed"):
        return f'<span class="ok">{esc}</span>'
    return esc

def align(lines):
    """Pad consecutive tab-separated rows into aligned columns (a <pre> would use 8-column tab stops)."""
    out, block = [], []
    def flush():
        if block:
            cells = [r.split("\t") for r in block]
            w = [max(len(c[i]) for c in cells if i < len(c)) for i in range(max(map(len, cells)))]
            out.extend("  ".join(c.ljust(w[i]) for i, c in enumerate(row)).rstrip() for row in cells)
            block.clear()
    for l in lines:
        if "\t" in l:
            block.append(l)
        else:
            flush()
            out.append(l)
    flush()
    return out

raw = open(os.path.join(site, "transcript.txt")).read().rstrip("\n").splitlines()
transcript = "\n".join(colour(l) for l in align(raw))
css = open(os.path.join(site, "style.css")).read()
page = (
    open(os.path.join(site, "template.html")).read()
    .replace("@@CSS@@", css)
    .replace("@@TOOLS@@", "\n".join(rows))
    .replace("@@TRANSCRIPT@@", transcript)
    .replace("@@REPO@@", repo)
    .replace("@@NAMES@@", ", ".join(names))
    .replace("@@NAMES_PROSE@@", ", ".join(names[:-1]) + " and " + names[-1])
)
assert "@@" not in page
os.makedirs(os.path.join(root, "docs"), exist_ok=True)
open(os.path.join(root, "docs", "index.html"), "w").write(page)
print("wrote docs/index.html with tools:", ", ".join(names))
