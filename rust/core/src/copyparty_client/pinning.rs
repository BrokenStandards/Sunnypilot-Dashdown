//! A TOFU TLS verifier for the comma's self-signed copyparty cert.
//!
//! copyparty serves HTTPS on the same port as HTTP, with a self-signed cert that
//! either is the bundled (shared) "insecure" cert or, when the device has cfssl,
//! a per-device cert it **regenerates on network change**. So we cannot validate
//! against a CA, and the leaf fingerprint is not stable across IP changes — the
//! stable identity is the server hostname (see [`crate::identity`]). This
//! verifier therefore **accepts any presented chain** and merely **captures the
//! leaf's SHA-256** into a shared cell; the trust decision (hostname match,
//! tolerate cert rotation) is made by [`crate::identity`] using the captured
//! fingerprint. Signature checks still use the ring algorithms (a real TLS
//! session), so this only removes CA/hostname validation, not channel integrity.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{
    ring, verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};
use sha2::{Digest, Sha256};

/// Shared slot the verifier writes the most-recent leaf fingerprint into.
pub type CertCapture = Arc<Mutex<Option<[u8; 32]>>>;

#[derive(Debug)]
pub struct PinningVerifier {
    capture: CertCapture,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let digest = Sha256::digest(end_entity.as_ref());
        *self.capture.lock().unwrap() = Some(digest.into());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// Build a rustls client config whose verifier captures the leaf fingerprint
/// into `capture` and accepts any self-signed chain (TOFU). The process crypto
/// provider must already be installed ([`crate::tls::ensure_crypto_provider`]).
pub fn pinning_client_config(capture: CertCapture) -> rustls::ClientConfig {
    let algs = ring::default_provider().signature_verification_algorithms;
    let verifier = PinningVerifier { capture, algs };
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth()
}

/// Lowercase hex of a 32-byte fingerprint.
pub fn hex_sha256(fp: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in fp {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{ServerName, UnixTime};

    /// The verifier records the SHA-256 of the presented leaf into the cell.
    #[test]
    fn captures_leaf_fingerprint() {
        crate::tls::ensure_crypto_provider();
        let cert = rcgen::generate_simple_self_signed(vec!["comma-test".into()]).unwrap();
        let der = CertificateDer::from(cert.cert.der().to_vec());
        let expected: [u8; 32] = Sha256::digest(der.as_ref()).into();

        let capture: CertCapture = Arc::new(Mutex::new(None));
        let algs = ring::default_provider().signature_verification_algorithms;
        let v = PinningVerifier {
            capture: capture.clone(),
            algs,
        };
        let sn = ServerName::try_from("127.0.0.1").unwrap();
        v.verify_server_cert(&der, &[], &sn, &[], UnixTime::now())
            .unwrap();

        assert_eq!(capture.lock().unwrap().unwrap(), expected);
        assert_eq!(hex_sha256(&expected).len(), 64);
    }
}
