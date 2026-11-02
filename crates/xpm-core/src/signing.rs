use std::path::Path;

use sequoia_openpgp::cert::CertParser;
use sequoia_openpgp::parse::stream::{
    DetachedVerifierBuilder, MessageLayer, MessageStructure, VerificationHelper,
};
use sequoia_openpgp::parse::Parse;
use sequoia_openpgp::policy::StandardPolicy;
use sequoia_openpgp::{Cert, KeyHandle};

use crate::error::{XpmError, XpmResult};

#[derive(Debug, Clone)]
pub enum VerifyOutcome {
    Good { key_id: String },
    UnknownKey,
    Bad { reason: String },
}

pub fn load_keyring(path: &Path) -> XpmResult<Vec<Cert>> {
    let data = std::fs::read(path)
        .map_err(|e| XpmError::SignatureError(format!("read keyring {}: {e}", path.display())))?;

    let parser = CertParser::from_bytes(&data)
        .map_err(|e| XpmError::SignatureError(format!("parse keyring {}: {e}", path.display())))?;

    let mut certs = Vec::new();
    for cert in parser {
        certs.push(cert.map_err(|e| XpmError::SignatureError(format!("keyring entry: {e}")))?);
    }

    Ok(certs)
}

pub fn verify_file(file_path: &Path, sig_path: &Path, certs: &[Cert]) -> XpmResult<VerifyOutcome> {
    let data = std::fs::read(file_path)
        .map_err(|e| XpmError::SignatureError(format!("read {}: {e}", file_path.display())))?;
    let sig = std::fs::read(sig_path)
        .map_err(|e| XpmError::SignatureError(format!("read {}: {e}", sig_path.display())))?;

    verify_detached(&data, &sig, certs)
}

/// Verify a detached signature with GnuPG against a binary keyring.
///
/// Sequoia's policy rejects some valid Arch packager keys ("no binding
/// signature at time ..."), while GnuPG verifies them. pacman itself uses
/// gpgme, so this is also the closest compatibility path.
pub fn verify_with_gpg(
    file_path: &Path,
    sig_path: &Path,
    keyring_path: &Path,
) -> XpmResult<VerifyOutcome> {
    let mut gpg = std::process::Command::new("gpg");
    gpg.args(["--batch", "--no-tty"]);
    // GnuPG 2.4 may run keyboxd and silently ignore an explicit --keyring.
    // When the configured keyring lives in a pacman-style home directory, use
    // --homedir so pubring.gpg (the same trust store pacman uses) is read.
    let keyring_home = keyring_path
        .parent()
        .filter(|dir| dir.join("pubring.gpg").is_file());
    if let Some(home) = keyring_home {
        gpg.arg("--homedir").arg(home);
    } else {
        gpg.args(["--no-default-keyring", "--keyring"])
            .arg(keyring_path);
    }
    let gpg = gpg
        .arg("--verify")
        .arg(sig_path)
        .arg(file_path)
        .output()
        .map_err(|e| XpmError::SignatureError(format!("failed to run gpg: {e}")))?;

    if gpg.status.success() {
        return Ok(VerifyOutcome::Good {
            key_id: "gpg".to_string(),
        });
    }

    let stderr = String::from_utf8_lossy(&gpg.stderr);
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

fn verify_detached(data: &[u8], sig_bytes: &[u8], certs: &[Cert]) -> XpmResult<VerifyOutcome> {
    let policy = StandardPolicy::new();
    let helper = VHelper::new(certs);

    let mut verifier = DetachedVerifierBuilder::from_bytes(sig_bytes)
        .map_err(|e| XpmError::SignatureError(format!("parse signature: {e}")))?
        .with_policy(&policy, None, helper)
        .map_err(|e| XpmError::SignatureError(format!("init verifier: {e}")))?;

    verifier
        .verify_bytes(data)
        .map_err(|e| XpmError::SignatureError(format!("verify: {e}")))?;

    let helper = verifier.into_helper();
    Ok(helper.result.unwrap_or_else(|| VerifyOutcome::Bad {
        reason: "no signature found".to_string(),
    }))
}

struct VHelper {
    certs: Vec<Cert>,
    result: Option<VerifyOutcome>,
}

impl VHelper {
    fn new(certs: &[Cert]) -> Self {
        Self {
            certs: certs.to_vec(),
            result: None,
        }
    }
}

impl VerificationHelper for VHelper {
    fn get_certs(&mut self, _ids: &[KeyHandle]) -> sequoia_openpgp::Result<Vec<Cert>> {
        Ok(self.certs.clone())
    }

    fn check(&mut self, structure: MessageStructure) -> sequoia_openpgp::Result<()> {
        for layer in structure {
            if let MessageLayer::SignatureGroup { results } = layer {
                for result in results {
                    match result {
                        Ok(good) => {
                            self.result = Some(VerifyOutcome::Good {
                                key_id: good.ka.key().keyid().to_hex(),
                            });
                            return Ok(());
                        }
                        Err(e) => {
                            let msg = format!("{e}");
                            if msg.contains("no binding signature") {
                                self.result = Some(VerifyOutcome::UnknownKey);
                            } else {
                                self.result = Some(VerifyOutcome::Bad { reason: msg });
                            }
                        }
                    }
                }
            }
        }

        if self.result.is_none() {
            self.result = Some(VerifyOutcome::Bad {
                reason: "no signature results".to_string(),
            });
        }

        match &self.result {
            Some(VerifyOutcome::Good { .. }) => Ok(()),
            Some(VerifyOutcome::Bad { reason }) => Err(anyhow::anyhow!("bad signature: {reason}")),
            Some(VerifyOutcome::UnknownKey) => Err(anyhow::anyhow!("unknown signing key")),
            None => Err(anyhow::anyhow!("no verification result")),
        }
    }
}
