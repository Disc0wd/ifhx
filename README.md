# ifhx

*I fucking hate x. The install manager for everything you fucking hate.*

Installs and updates the IFHX tool suite ([ifhf](https://github.com/Disc0wd/ifhf) and whatever
comes next). One command to get a tool, one command to keep everything current, including `ifhx`
itself. If you allow it, it also installs programs from other GitHub releases
(`ifhx add owner/name`).

```console
$ ifhx install ifhf
┃ ifhf
┃ Version   0.1.0
┃ Method    prebuilt binary (x86_64-linux), signature and checksum verified
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

To also check the signature (what `ifhx` itself does for every later update):

```bash
curl -fsSLO https://github.com/Disc0wd/ifhx/releases/latest/download/ifhx-x86_64-linux.minisig
minisign -Vm ifhx-x86_64-linux -P RWQW5yVx+QbbzzAcSLmwTj9u80DbNZpgi2MUKLThbjWAHY31e72Y3vCM
```

After that, `ifhx update` keeps it current. Or build from source: `cargo install --git https://github.com/Disc0wd/ifhx`.

## Commands

| Command | What it does |
|:---|:---|
| `ifhx install <tool>...` | Install tools from a verified prebuilt binary (`--source` builds from source instead) |
| `ifhx add <owner/name>` | Install a program from another GitHub release (off by default, see below) |
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

## Adding other programs

Off by default. Switch it on in `~/.config/ifhx/config.toml`:

```toml
[registry]
allow_foreign = true
```

Then:

```console
$ ifhx add BurntSushi/ripgrep
rg
  From:      github.com/BurntSushi/ripgrep
  Version:   16.0.0
  File:      ripgrep-16.0.0-x86_64-unknown-linux-musl.tar.gz
  Check:     matches the SHA-256 digest GitHub recorded for the file
  Trust:     not an IFHX tool; ifhx does not vet its publisher
  Install:   ~/.local/bin/rg
```

`ifhx` asks before installing. After that `ifhx update`, `list`, `info` and `remove` treat it like
any other tool (`list` shows it as `github`). Switching `allow_foreign` off again stops updating
these tools; `remove` still works.

- **What is checked:** the download must match the SHA-256 GitHub recorded for the file, or a
  checksum file in the same release. If the publisher gives neither, the plan says **UNVERIFIED** and
  `ifhx` installs or updates it only after you answer yes at a terminal; `--yes` does not apply. A
  tool that was verified when added is never updated to an unverified release.
- **What is not checked:** whether the publisher is honest or the program safe. These programs are
  not signed with the IFHX key, and a checksum published next to a file only shows the download
  arrived intact.
- **What is supported:** one program per repository, shipped for Linux as a plain executable or a
  `.tar.gz`. If several files or executables could be meant, `ifhx` lists them and stops. AppImages
  belong to [ifhai](https://github.com/Disc0wd/ifhai). Nothing added this way is built from source.
- An added program can never take the name of an IFHX tool, replace a file `ifhx` did not install, or
  use the name of a command that already exists on your `PATH` or in the system directories.

## How it works

- **Registry.** `registry.toml` in this repository lists the tools. ifhx fetches it from GitHub
  and caches it for an hour. Layers, in order: fresh cache, network, stale cache, and a copy built
  into the binary, so it works offline. A new tool appears for everyone without a new ifhx release.
- **Versions.** The latest release is found by following GitHub's `/releases/latest` redirect:
  no API, no token, no rate limit. A redirect that lands on a *different* repository (renamed,
  transferred, or a name someone else claimed) is refused. Only `ifhx add` and updating added tools
  use the GitHub API, which allows 60 requests an hour without a token.
- **Install.** The release asset `<tool>-<platform>` is downloaded next to its destination, checked
  against `<tool>-<platform>.sha256` and against its signature `<tool>-<platform>.minisig`, and only
  then moved into place with a single `rename`. A failed download, a failed check or a crash leaves
  the previous version untouched.
- **Source builds are opt-in.** If a release has no binary for your platform, ifhx says so and stops.
  `--source` clones the tag and runs `cargo build --release --locked` in a private temporary
  directory. That needs `git` and `cargo`, and it runs the project's own build scripts, so it never
  happens automatically, not even with `--yes`.
- **Nothing it did not install is overwritten.** If the destination already holds a file ifhx did not
  put there, or one you changed after it was installed, ifhx leaves it alone and tells you.
- **Self-update.** `ifhx` is a tool like any other. Replacing the running binary is safe on Linux
  because the swap is a `rename`.
- **State.** What is installed is recorded in `~/.local/state/ifhx/installed.toml`. A damaged file
  is reported, never silently reset.
- **Terminal output** follows [ifhf](https://github.com/Disc0wd/ifhf): colour and symbols on a
  terminal, plain stable text when piped, `NO_COLOR` respected, ASCII fallback on non-UTF-8 locales.

## Trust model

Be clear about what is and is not protected:

- Everything is fetched over HTTPS (`curl --proto =https --tlsv1.2`).
- **IFHX tools are signed.** A binary is installed only if its [minisign](https://jedisct1.github.io/minisign/)
  signature verifies against the public key built into `ifhx` and names exactly that file and release
  tag. Taking over a repository, a release or the registry is therefore not enough to ship you a
  binary, and an old signed release cannot be replayed under a new tag. The secret key is kept
  offline by the maintainer, never in CI.

  ```
  RWQW5yVx+QbbzzAcSLmwTj9u80DbNZpgi2MUKLThbjWAHY31e72Y3vCM
  ```
- **What signing does not cover.** CI builds the binaries and the maintainer signs what CI built, so
  a compromised build pipeline could get a bad binary signed. `--source` builds are not signed. The
  upgrade from `ifhx` 0.1.0 to 0.2.0 is performed by the old version, which checks the checksum only.
- The registry decides which repositories ifhx looks at. It comes from this repository's `main`
  branch. It can name a repository and nothing else that matters: not the binary's name, not an asset,
  not a command to run. Its text must be a single plain line, because it is shown on your terminal.
- Responses are size-limited (1 MiB for text, 512 MiB for a download); a server that sends more is
  stopped.
- Source builds (`--source` only) run the project's own build scripts as you. Only build projects you trust.
- Programs added with `ifhx add` are outside all of the above except HTTPS, checksums and size limits.

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
allow_foreign = false     # true: allow `ifhx add owner/name`
```

`IFHX_REGISTRY` overrides the registry URL for one run. ifhx warns if the install directory is not
on your `PATH`.

## Adding a tool to the suite

1. Publish GitHub Releases tagged `vX.Y.Z` whose assets are `<bin>-x86_64-linux` and
   `<bin>-x86_64-linux.sha256` (`sha256sum` format). The workflow in `.github/workflows/release.yml`
   does exactly that for a Rust project: push a tag that matches `Cargo.toml`.
2. Sign the release. The workflow only creates a **draft**, which users cannot see. Run this from
   an `ifhx` checkout, on your own machine; it checks the checksum, asks for your key's passphrase,
   uploads the signature and publishes:

   ```bash
   scripts/sign-release.sh <tool> <tag>
   ```

   It first lists the commits and files changed since the previous release and asks you to confirm
   they are yours. Read them: the signature vouches for whatever CI built from them.

3. Add a table to `registry.toml`:

   ```toml
   [tools.ifhz]
   repo = "Disc0wd/ifhz"
   description = "What it does"
   ```

   The asset names follow from the table's name.

## Development

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Network, source builds and signature checks are behind three traits (`Fetcher`, `Builder`,
`Verifier`) and tested with fakes; the real signature check is tested against signatures made by
`minisign`, and archive unpacking against hostile archives. Tests never touch the network or your real
install directory.

## Limitations

- Linux only. Release assets are currently built for x86_64; `aarch64` works for tools that
  publish signed `<tool>-aarch64-linux` assets, and via source builds.
- `ifhx add` does not unpack `.zip`, `.tar.xz` or `.tar.zst`, and installs one program per repository.

## Licence

[MIT](LICENSE). Use, copy, modify and redistribute this code for any purpose, including commercial
use, as long as the copyright notice and licence text stay with it (that notice is the credit).
