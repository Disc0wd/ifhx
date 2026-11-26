#!/usr/bin/env bash
# Sign one release of an IFHX tool with your minisign key, upload the signature, and publish the
# release if it is still a draft. Run it yourself, in your own terminal: minisign asks for the
# passphrase.
#
#   scripts/sign-release.sh <tool> <tag>      e.g.  scripts/sign-release.sh ifhx v0.2.0
#
# IFH_KEY overrides the secret key path (default ~/.minisign/ifh.key).
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 <tool> <tag>" >&2
  exit 2
fi
tool=$1 tag=$2
repo="Disc0wd/$tool"
key=${IFH_KEY:-$HOME/.minisign/ifh.key}
here=$(cd "$(dirname "$0")/.." && pwd)
# The public key ifhx trusts, so a signature made with the wrong key is caught before uploading.
pub=${IFH_PUBKEY:-$(grep -o 'RW[A-Za-z0-9+/]\{54\}' "$here/src/verify.rs" | head -1)}

for need in gh minisign sha256sum; do
  command -v "$need" >/dev/null || { echo "$need is not installed" >&2; exit 1; }
done
[ -f "$key" ] || { echo "no secret key at $key" >&2; exit 1; }
[ -n "$pub" ] || { echo "no pinned public key found in src/verify.rs" >&2; exit 1; }

draft=$(gh release view "$tag" -R "$repo" --json isDraft --jq .isDraft)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
gh release download "$tag" -R "$repo" -D "$work" -p "$tool-*-linux" -p "$tool-*-linux.sha256"

shopt -s nullglob
bins=("$work/$tool"-*-linux)
[ ${#bins[@]} -gt 0 ] || { echo "the release has no $tool-*-linux binary" >&2; exit 1; }

for bin in "${bins[@]}"; do
  name=$(basename "$bin")
  (cd "$work" && sha256sum -c "$name.sha256")
  echo "Signing \"$name $tag\" from $repo (draft: $draft)"
  sha256sum "$bin" | cut -d' ' -f1
  # The signed comment names the file and tag; ifhx refuses a signature that names anything else.
  minisign -S -s "$key" -m "$bin" -x "$bin.minisig" -t "$name $tag"
  minisign -V -q -P "$pub" -m "$bin" -x "$bin.minisig" ||
    { echo "that key is not the one ifhx trusts; nothing was uploaded" >&2; exit 1; }
done

gh release upload "$tag" -R "$repo" "$work"/*.minisig --clobber
if [ "$draft" = true ]; then
  gh release edit "$tag" -R "$repo" --draft=false --latest
  echo "Published $repo $tag."
else
  echo "Added signatures to $repo $tag."
fi
