#!/usr/bin/env python3
"""Check that the IFHX family sites follow SITE-SPEC.md. Run from anywhere; sibling repos sit next to ifhx/."""
import hashlib, os, re, sys

base = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
tools = ["ifhf", "ifhai", "ifhx"]
bad = []

def fail(tool, msg):
    bad.append(f"{tool}: {msg}")

css = {}
pages = {}
for t in tools:
    css[t] = hashlib.sha256(open(f"{base}/{t}/site/style.css", "rb").read()).hexdigest()
    pages[t] = open(f"{base}/{t}/docs/index.html", encoding="utf-8").read()
if len(set(css.values())) != 1:
    bad.append("style.css differs between repos: " + ", ".join(f"{t}={h[:8]}" for t, h in css.items()))

for t, p in pages.items():
    nav = re.search(r'<nav class="family"[^>]*>(.*?)</nav>', p, re.S)
    if not nav:
        fail(t, "no family nav in the top bar"); continue
    links = re.findall(r'<a href="([^"]+)"([^>]*)>([^<]+)</a>', nav.group(1))
    if [l[2] for l in links] != tools + ["Source"]:
        fail(t, f"family nav is {[l[2] for l in links]}")
    cur = [l[2] for l in links if "aria-current" in l[1]]
    if cur != [t]:
        fail(t, f"aria-current is {cur}, expected [{t!r}]")
    if p.count("<h1") != 1: fail(t, "needs exactly one h1")
    hero = p[p.index('<div class="hero">'):p.index("<section")]
    if 'class="get"' not in hero: fail(t, "install block is not in the hero")
    if p.count('class="get"') != 1: fail(t, "install block appears more than once")
    for h in re.findall(r"<h2[^>]*>(.*?)</h2>", p):
        if not h.endswith("."): fail(t, f"h2 does not end in a full stop: {h!r}")
    if "<h2 id=\"h-safe\">Careful by default.</h2>" not in p and "<h2 id=\"h-rules\">Careful by default.</h2>" not in p:
        fail(t, "missing the 'Careful by default.' section")
    if 'class="rules"' not in p: fail(t, "safety section is not ul.rules")
    foot = re.search(r"<footer>(.*?)</footer>", p, re.S).group(1)
    if "MIT licensed. Written in Rust. Part of the IFHX family." not in foot: fail(t, "footer text differs")
    flinks = re.findall(r"<a [^>]*>([^<]+)</a>", foot)
    if flinks[:3] != ["Source", "README", "Licence"] or len(flinks) > 5: fail(t, f"footer links are {flinks}")
    if any(n in flinks for n in ("ifhf", "ifhai", "ifhx")): fail(t, "family links belong in the top bar, not the footer")
    if re.search(r'https?://(?!disc0wd\.github\.io|github\.com/Disc0wd|raw\.githubusercontent\.com/Disc0wd)[^"\' )<]+', re.sub(r"<pre.*?</pre>", "", p, flags=re.S)):
        fail(t, "page references an external host")
    title = re.search(r"<title>(.*?)</title>", p).group(1)
    if not re.match(rf"{t}: I fucking hate .+ \(.+\)$", title): fail(t, f"title pattern: {title!r}")

if bad:
    print("FAMILY SITES DIFFER FROM SITE-SPEC.md:"); print("\n".join("  " + b for b in bad)); sys.exit(1)
print("family sites consistent:", ", ".join(tools))
