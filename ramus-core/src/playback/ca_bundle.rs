//! The CA file libmpv verifies HTTPS streams against.
//!
//! mpv checks a server's certificate only when `tls-verify` is on, and
//! the FFmpeg TLS backend behind it then needs a list of trusted roots.
//! No libmpv the app runs against can be relied on to find one by itself:
//!
//! - macOS: Homebrew's OpenSSL looks in its own prefix
//!   (`/opt/homebrew/etc/openssl@…/cert.pem`), which exists only where
//!   Homebrew is installed.
//! - Windows: the DLL's static OpenSSL carries the path of the machine it
//!   was built on.
//! - Linux: the AppImage's GnuTLS reads the Debian bundle path, which
//!   other distributions don't provide.
//! - iOS: GnuTLS has no system trust store there.
//! - Android: mbedTLS has none anywhere.
//!
//! Every one of those backends reads a PEM file when given one, so the app
//! writes the Mozilla root list to its data directory at startup and
//! points mpv's `tls-ca-file` at it. The roots come from
//! `webpki-root-certs`, the certificate form of the `webpki-roots` list
//! reqwest already trusts for every Plex API call, so a server the app
//! can talk to is a server mpv can stream from.

use std::io;
use std::path::{Path, PathBuf};

use base64::Engine;

/// File name of the bundle inside the app's data directory.
pub const CA_BUNDLE_FILE: &str = "mpv-ca-roots.pem";

/// PEM bodies wrap at 64 characters (RFC 7468), i.e. 48 bytes per line.
const PEM_LINE_BYTES: usize = 48;

/// The Mozilla root certificates as one concatenated PEM document.
pub fn ca_bundle_pem() -> String {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut pem = String::new();
    for cert in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
        pem.push_str("-----BEGIN CERTIFICATE-----\n");
        for chunk in cert.as_ref().chunks(PEM_LINE_BYTES) {
            engine.encode_string(chunk, &mut pem);
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
    }
    pem
}

/// Write the bundle into `dir` unless an identical copy is already there,
/// and return its path.
///
/// The file is replaced by rename, so a reader never sees a partial list.
/// A copy left by an earlier version is kept when the new one can't be
/// written: an older root list still verifies, a missing one doesn't.
pub fn ensure_ca_bundle(dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join(CA_BUNDLE_FILE);
    let pem = ca_bundle_pem();
    if std::fs::read(&path).is_ok_and(|existing| existing == pem.as_bytes()) {
        return Ok(path);
    }
    let written = (|| {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{CA_BUNDLE_FILE}.tmp"));
        std::fs::write(&tmp, &pem)?;
        std::fs::rename(&tmp, &path)
    })();
    match written {
        Ok(()) => Ok(path),
        Err(e) if path.is_file() => {
            log::warn!("could not refresh the mpv CA bundle, keeping the existing one: {e}");
            Ok(path)
        }
        Err(e) => Err(e),
    }
}

/// The bundle's path in the app's data directory, written on demand.
///
/// `None` means the file could not be written. Callers still turn
/// `tls-verify` on: HTTPS streams then fail to open instead of playing
/// unverified, and plain-HTTP and local-file playback are unaffected.
pub fn mpv_ca_file() -> Option<PathBuf> {
    let dir = match crate::plex::token_store::config_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("no data directory for the mpv CA bundle: {e}");
            return None;
        }
    };
    match ensure_ca_bundle(&dir) {
        Ok(path) => Some(path),
        Err(e) => {
            log::error!("could not write the mpv CA bundle: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn decode_pem(pem: &str) -> Vec<Vec<u8>> {
        let engine = base64::engine::general_purpose::STANDARD;
        let mut certs = Vec::new();
        let mut body: Option<String> = None;
        for line in pem.lines() {
            match line {
                "-----BEGIN CERTIFICATE-----" => body = Some(String::new()),
                "-----END CERTIFICATE-----" => {
                    let b64 = body.take().expect("END without BEGIN");
                    certs.push(engine.decode(b64).expect("valid base64"));
                }
                _ => {
                    assert!(line.len() <= 64, "PEM line longer than 64 characters");
                    body.as_mut()
                        .expect("text outside a certificate")
                        .push_str(line);
                }
            }
        }
        assert!(body.is_none(), "unterminated certificate");
        certs
    }

    #[test]
    fn pem_round_trips_every_root() {
        let decoded = decode_pem(&ca_bundle_pem());
        let roots = webpki_root_certs::TLS_SERVER_ROOT_CERTS;
        assert!(roots.len() > 100, "implausibly short root list");
        assert_eq!(decoded.len(), roots.len());
        for (pem, der) in decoded.iter().zip(roots) {
            assert_eq!(pem.as_slice(), der.as_ref());
        }
    }

    /// mpv must trust exactly the roots reqwest does. The two crates are
    /// published together from one source list; this fails when a
    /// dependency update moves one without the other.
    #[test]
    fn bundle_matches_the_roots_reqwest_trusts() {
        let key = |subject: &[u8], spki: &[u8]| (subject.to_vec(), spki.to_vec());
        let bundled: BTreeSet<_> = webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|der| {
                let anchor = webpki::anchor_from_trusted_cert(der).expect("root parses");
                key(
                    anchor.subject.as_ref(),
                    anchor.subject_public_key_info.as_ref(),
                )
            })
            .collect();
        let trusted: BTreeSet<_> = webpki_roots::TLS_SERVER_ROOTS
            .iter()
            .map(|a| key(a.subject.as_ref(), a.subject_public_key_info.as_ref()))
            .collect();
        assert_eq!(
            bundled, trusted,
            "webpki-root-certs and webpki-roots are on different versions"
        );
    }

    #[test]
    fn ensure_writes_once_and_repairs_a_stale_copy() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("data");

        let path = ensure_ca_bundle(&nested).unwrap();
        assert_eq!(path, nested.join(CA_BUNDLE_FILE));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ca_bundle_pem());
        assert!(!nested.join(format!("{CA_BUNDLE_FILE}.tmp")).exists());

        let first_write = std::fs::metadata(&path).unwrap().modified().unwrap();
        ensure_ca_bundle(&nested).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            first_write,
            "an identical bundle is left alone"
        );

        std::fs::write(&path, "stale").unwrap();
        ensure_ca_bundle(&nested).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ca_bundle_pem());
    }
}
