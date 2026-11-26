//! Signature check for IFHX releases. Every IFHX binary is signed offline with the maintainer's
//! minisign key; ifhx only installs one whose signature verifies against a key pinned in this file,
//! and whose signed ("trusted") comment names exactly the asset and tag being installed. The
//! comment binding stops a validly signed older or different binary being served under a new name.

use crate::error::{Error, Result};
use minisign_verify::{PublicKey, Signature};
use std::io::Read;
use std::path::Path;

/// Trusted public keys. A list so a new key can be added in one release and the old one retired in a
/// later one; a signature by any listed key is accepted.
pub const KEYS: &[&str] = &["RWQW5yVx+QbbzzAcSLmwTj9u80DbNZpgi2MUKLThbjWAHY31e72Y3vCM"];

/// What must be signed for `asset` at release `tag`.
pub fn comment(asset: &str, tag: &str) -> String {
    format!("{asset} {tag}")
}

pub trait Verifier {
    /// `file` must carry a valid signature `sig` (the text of a `.minisig`) whose trusted comment is `comment`.
    fn verify(&self, file: &Path, sig: &str, comment: &str) -> Result<()>;
}

pub struct Minisign;

impl Verifier for Minisign {
    fn verify(&self, file: &Path, sig: &str, comment: &str) -> Result<()> {
        verify_with(KEYS, file, sig, comment)
    }
}

fn bad(why: &str) -> Error {
    Error::Verify(format!(
        "The signature check failed ({why}), so nothing was installed."
    ))
}

pub fn verify_with(keys: &[&str], file: &Path, sig: &str, comment: &str) -> Result<()> {
    let sig = Signature::decode(sig).map_err(|_| bad("the signature file is not valid"))?;
    let mut failure = "it was not signed by a trusted key";
    for k in keys {
        let Ok(key) = PublicKey::from_base64(k) else {
            continue;
        };
        let Ok(mut v) = key.verify_stream(&sig) else {
            continue;
        };
        let mut f = std::fs::File::open(file)?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            v.update(&buf[..n]);
        }
        if v.finalize().is_err() {
            failure = "the file does not match its signature";
            continue;
        }
        // finalize() also authenticates the trusted comment, so it can be relied on here.
        if sig.trusted_comment() != comment {
            return Err(bad(&format!(
                "it is signed for \"{}\", not \"{comment}\"",
                sig.trusted_comment().escape_debug()
            )));
        }
        return Ok(());
    }
    Err(bad(failure))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made with a throwaway key by `minisign -S ... -t "ifhf-x86_64-linux v9.9.9"`; not a release key.
    const KEY: &str = "RWT4GlrGNZFuaG3gidTuD3RZIx2RuW1rAQ1ANf1LayvcoaoThtWUxd6R";
    const SIG: &str = "untrusted comment: signature from minisign secret key
RUT4GlrGNZFuaBNjle2ItxNkm9WcWVe94ClKPUCnUQneJDITNzSZgxw9f657BbLRbNyNGWTQKROY0qPGkjrtC44Vd9Vi3NLmqgs=
trusted comment: ifhf-x86_64-linux v9.9.9
6HndhfmQbjzBJErWOsTC+77AQiLow7k/pvm6+8qQAtRrAj0DmSzqvittQlri2wNK7R/2mIbPX2Pa76QRoajaDA==
";

    fn file(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        std::fs::write(&p, bytes).unwrap();
        (d, p)
    }

    #[test]
    fn a_good_signature_for_the_right_name_passes() {
        let (_d, p) = file(b"hello ifhx");
        verify_with(&[KEY], &p, SIG, "ifhf-x86_64-linux v9.9.9").unwrap();
    }

    #[test]
    fn a_signed_binary_under_another_name_or_tag_is_refused() {
        let (_d, p) = file(b"hello ifhx");
        for wrong in ["ifhf-x86_64-linux v9.9.8", "ifhai-x86_64-linux v9.9.9", ""] {
            let e = verify_with(&[KEY], &p, SIG, wrong).unwrap_err();
            assert!(e.to_string().contains("is signed for"), "{wrong}: {e}");
        }
    }

    #[test]
    fn a_changed_file_is_refused() {
        let (_d, p) = file(b"hello ifhx!");
        let e = verify_with(&[KEY], &p, SIG, "ifhf-x86_64-linux v9.9.9").unwrap_err();
        assert!(e.to_string().contains("does not match"), "{e}");
    }

    #[test]
    fn another_key_and_garbage_are_refused() {
        let (_d, p) = file(b"hello ifhx");
        // The pinned release key did not make this signature.
        assert!(verify_with(KEYS, &p, SIG, "ifhf-x86_64-linux v9.9.9").is_err());
        assert!(verify_with(&[], &p, SIG, "x").is_err());
        assert!(verify_with(&[KEY], &p, "not a signature", "x").is_err());
    }

    #[test]
    fn the_pinned_keys_parse() {
        for k in KEYS {
            PublicKey::from_base64(k).unwrap();
        }
    }
}
