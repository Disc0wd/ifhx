# ifhx

*I fucking hate x. The install manager for everything you fucking hate.*

Installs and updates the IFHX tool suite ([ifhf](https://github.com/Disc0wd/ifhf) and whatever
comes next). One command to get a tool, one command to keep everything current, including `ifhx`
itself.

```console
$ ifhx install ifhf
┃ ifhf
┃ Version   0.1.0
┃ Method    prebuilt binary (x86_64-linux), checksum verified
┃ Install   /home/you/.local/bin/ifhf

Install ifhf? [Y/n]

$ ifhx update
1 update(s) available:

Tool  Installed    Latest
ifhf  0.1.0     →  0.2.0
```

## Get ifhx

Download the release binary, check it, put it on your `PATH`:

```bash
curl -fsSLO https://github.com/Disc0wd/ifhx/releases/latest/download/ifhx-x86_64-linux
curl -fsSL  https://github.com/Disc0wd/ifhx/releases/latest/download/ifhx-x86_64-linux.sha256 | sha256sum -c -
install -Dm755 ifhx-x86_64-linux ~/.local/bin/ifhx
```

After that, `ifhx update` keeps it current. Or build from source: `cargo install --git https://github.com/Disc0wd/ifhx`.

## Commands

| Command | What it does |
|:---|:---|
| `ifhx install <tool>...` | Install tools: a prebuilt binary if one exists, otherwise built from source |
| `ifhx update [<tool>...]` | Update installed tools (default: all), and `ifhx` itself, last |
| `ifhx updates` | Show which tools have a newer release; changes nothing |
| `ifhx list` | Installed tools, versions, how they were installed, where |
| `ifhx available` | Every tool in the registry, and whether it is installed |
| `ifhx info <tool>...` | Details, installed version, latest release |
| `ifhx remove <tool>...` | Delete a tool's binary. Its own config and data are left alone |
| `ifhx sync` | Refresh the tool registry now |

Options: `--tag <tag>` (install a specific release), `--source` (build from source even if a binary
exists), `--yes` / `--noconfirm`, `--no-color`.

Exit status: `0` ok, `1` failure, `2` usage error, `3` not found, `5` confirmation required (use
`--yes` when not on a terminal), `6` cancelled.

## How it works

- **Registry.** `registry.toml` in this repository lists the tools. ifhx fetches it from GitHub
  and caches it for an hour. Layers, in order: fresh cache, network, stale cache, and a copy built
  into the binary, so it works offline. A new tool appears for everyone without a new ifhx release.
- **Versions.** The latest release is found by following GitHub's `/releases/latest` redirect:
  no API, no token, no rate limit.
- **Install.** The release asset `<bin>-<platform>` is downloaded next to its destination, its
  SHA-256 is checked against `<bin>-<platform>.sha256`, and it is moved into place with a single
  `rename`. A failed download, a bad checksum or a crash leaves the previous version untouched.
- **Source fallback.** If a release has no binary for your platform (or has no release at all),
  ifhx clones the tag and runs `cargo build --release`. That needs `git` and `cargo`.
- **Self-update.** `ifhx` is a tool like any other. Replacing the running binary is safe on Linux
  because the swap is a `rename`.
- **State.** What is installed is recorded in `~/.local/state/ifhx/installed.toml`. A damaged file
  is reported, never silently reset.
- **Terminal output** follows [ifhf](https://github.com/Disc0wd/ifhf): colour and symbols on a
  terminal, plain stable text when piped, `NO_COLOR` respected, ASCII fallback on non-UTF-8 locales.

## Trust model

Be clear about what is and is not protected:

- Everything is fetched over HTTPS (`curl --proto =https --tlsv1.2`).
- The SHA-256 check catches corrupted and truncated downloads. The checksum is published in the
  same release as the binary, so it does **not** protect against a compromised repository or
  release. Signed releases would be the next step.
- The registry decides which repositories ifhx will install from. It comes from this repository's
  `main` branch; treat write access to it accordingly. Entries are validated (names, `owner/name`
  repos, asset names) before use and a malformed registry is never cached.
- Source builds run the registry's build command (`cargo build --release`) in a clone of the tag.

## Configuration

`~/.config/ifhx/config.toml`, all optional:

```toml
[general]
color = true
confirm = true            # false: do not ask before installing/updating/removing

[install]
# dir = "/home/you/.local/bin"   # where tools go (absolute path; default ~/.local/bin)

[registry]
url = "https://raw.githubusercontent.com/Disc0wd/ifhx/main/registry.toml"
max_age_hours = 1
```

`IFHX_REGISTRY` overrides the registry URL for one run. ifhx warns if the install directory is not
on your `PATH`.

## Adding a tool to the suite

1. Publish GitHub Releases tagged `vX.Y.Z` whose assets are `<bin>-x86_64-linux` and
   `<bin>-x86_64-linux.sha256` (`sha256sum` format). The workflow in `.github/workflows/release.yml`
   does exactly that for a Rust project: push a tag that matches `Cargo.toml`.
2. Add a table to `registry.toml`:

   ```toml
   [tools.ifhz]
   repo = "Disc0wd/ifhz"
   description = "What it does"
   # optional: bin, build, asset
   ```

## Development

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

All network and source-build behaviour is behind two traits (`Fetcher`, `Builder`) and tested
with fakes: verified install, corrupted and malformed downloads, source fallback, update ordering,
self-update, partial failures, every registry layer, and presentation. Tests never touch the
network or your real install directory.

## Limitations

- Linux only. Release assets are currently built for x86_64; `aarch64` works for tools that
  publish `<bin>-aarch64-linux` assets, and via source builds.
- No signature verification yet (see the trust model).

## Licence

[MIT](LICENSE). Use, copy, modify and redistribute this code for any purpose, including commercial
use, as long as the copyright notice and licence text stay with it (that notice is the credit).
