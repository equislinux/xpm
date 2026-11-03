//! OpenPGP verification through GnuPG, matching pacman's keyring.
//!
//! pacman itself uses gpgme; using the `gpg` binary against the pacman
//! keyring keeps the trust store and policy identical without pulling a second
//! OpenPGP implementation into xpm.

use std::path::Path;

use crate::error::{XpmError, XpmResult};

#[derive(Debug, Clone)]
pub enum VerifyOutcome {
    Good { key_id: String },
    UnknownKey,
    Bad { reason: String },
}

/// Verify a detached signature with GnuPG.
///
/// GnuPG 2.4 may run keyboxd and silently ignore an explicit `--keyring`.
/// When the configured keyring lives in a pacman-style home directory, use
/// `--homedir` so `pubring.gpg` is read.
pub fn verify_file(
    file_path: &Path,
    sig_path: &Path,
    keyring_path: &Path,
) -> XpmResult<VerifyOutcome> {
    let mut gpg = std::process::Command::new("gpg");
    gpg.args(["--batch", "--no-tty"]);
    let keyring_home = keyring_path
        .parent()
        .filter(|dir| dir.join("pubring.gpg").is_file());
    if let Some(home) = keyring_home {
        gpg.arg("--homedir").arg(home);
    } else {
        gpg.args(["--no-default-keyring", "--keyring"])
            .arg(keyring_path);
    }
    let output = gpg
        .arg("--verify")
        .arg(sig_path)
        .arg(file_path)
        .output()
        .map_err(|e| XpmError::SignatureError(format!("failed to run gpg: {e}")))?;

    if output.status.success() {
        return Ok(VerifyOutcome::Good {
            key_id: "gpg".to_string(),
        });
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("NO_PUBKEY") || stderr.contains("Can't check signature") {
        return Ok(VerifyOutcome::UnknownKey);
    }
    Ok(VerifyOutcome::Bad {
        reason: stderr
            .lines()
            .next()
            .unwrap_or("gpg verification failed")
            .to_string(),
    })
}
