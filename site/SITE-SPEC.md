# IFHX family sites: unified specification

Every IFHX tool has a one-page GitHub Pages site. This is the contract they all follow, so a visitor
who knows one knows all of them. The canonical copy of this file and of `style.css` lives in the
`ifhx` repository. `site/check_family.py` enforces the checkable parts.

## 1. Page anatomy (fixed order)

1. **Top bar**
2. **Hero:** headline, lede, demo, demo note, install block
3. **Body sections** (slot order in §4)
4. **Footer**

One `<main>`, one `<h1>`. Every body section is `<section aria-labelledby="h-…">` with an `<h2>` that
is a full sentence ending in a full stop.

## 2. Top bar

- Left: wordmark (`a.mark`, mono, links to `#`) and the tagline (`span.aka`), e.g. *I fucking hate Flatpak*.
- Right: **the family nav, identical on every site**: `ifhf`, `ifhai`, `ifhx`, `Source`, in that order
  (add new tools after `ifhx`, before `Source`). The current site carries `aria-current="page"`.
- No project-specific links here. They belong in the footer.
- Below 40rem the bar stacks (brand over nav).

## 3. Hero

| Part | Rule |
|:---|:---|
| `h1` | The outcome the user gets, at most seven words, ends with a full stop |
| `.lede` | One or two sentences; the tool's name in `<b>` |
| Demo | Real captured output only. `.race` (two panes, before/after) when there is a measured comparison, `.solo` (one pane) otherwise |
| Demo note | One line under the demo (`.race-note`) saying what the output is and where it came from |
| Install block | `.get`, **inside the hero**, in this order: with `ifhx` first, then the direct download with checksum, then one `.fine` line (`cargo install --git …`, platform note). `ifhx` itself shows the direct download only |

## 4. Body sections (slot order)

Use the slots that apply, in this order. A site may skip a slot, never reorder them.

1. **What it does / how it works.** `.two` (paired statements), `.split` (prose + terminal) or `.tools`
2. **Proof.** Measured numbers: `.rows` chart or `.tbl`. Include where the tool loses. No unmeasured claims
3. **Reference.** Commands, syntax or the tool list: `.tbl`, `.syn` or `.tools`
4. **Careful by default.** The safety section. **Always** this heading, **always** `ul.rules`
   (`<li><b>Short rule.</b><span>Why.</span></li>`), always including what is not protected
5. **Extend.** Only where relevant, and only for users (for example, `ifhx add`). How to publish or
   contribute a tool belongs in the README, never on a site

## 4.1 Footer

- Left: `MIT licensed. Written in Rust. Part of the IFHX family.`
- Right (`nav`): `Source`, `README`, `Licence`, then at most two project documents (for example
  `The data`, `Design decisions`, `Full benchmarks`). **No family links** (those are in the top bar).

## 5. Look

One stylesheet, `site/style.css`, **byte-identical in every repository**, inlined into the page at
build time. Change it in `ifhx`, copy it everywhere, run the check.

- **Type:** Schibsted Grotesk (text) and JetBrains Mono (code, wordmark, numbers), both self-hosted
  from `fonts/`. No third-party requests of any kind.
- **Colour:** the CSS variables in `:root`, light and dark through `prefers-color-scheme`. Accent blue
  for links and the tool's own results, `--warn` red only for losses and real warnings, terminal panes
  always dark.
- **Layout:** one `.wrap` column (max 70rem), breakpoints at 56rem (columns collapse) and 40rem (top
  bar stacks).
- **Motion:** none except the optional `ifhf` race clock; `prefers-reduced-motion` always respected.

## 6. Content rules

- Title: `name: I fucking hate X (short descriptor)`.
- Output shown is real output from a real run; numbers come from a measurement file in the repo.
- State limits plainly (unverified paths, platforms, what a checksum does not prove).
- No tracking, analytics, cookies or external resources. The sites collect nothing.

## 7. Accessibility

`lang`, landmarks (`header`/`nav`/`main`/`footer` via the structure above), `aria-current` on the
current site, visible focus, decorative clocks `aria-hidden`, charts carry an `aria-label` that says
the numbers, nothing conveyed by colour alone, readable at 200% zoom and at 320px width, print-safe.

## 8. Generation and checks

Each repo has a generator that fills one template with `@@CSS@@`, `@@REPO@@` and its own data, and
writes `docs/index.html`. `python3 site/check_family.py` (run from `ifhx`, with the sibling repos
next to it) verifies: identical CSS, identical family nav apart from `aria-current`, the footer
pattern, one `h1`, section headings ending in a full stop, the `Careful by default.` section, and
that the install block is in the hero.

## 9. Adding a site for a new tool

Copy `site/style.css`, write `template.html` following §1 to §4, add the tool to the family nav in
**all** sites and to `registry.toml`, rebuild every site, run the check.
