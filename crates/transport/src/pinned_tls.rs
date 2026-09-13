//! Preserve standard certificate/time/signature verification against the saved
//! certificate name while allowing its network address to change. Discovery is
//! never an authority for TLS trust. Reject even a different leaf signed by the pin.
use photobridge_core::{Error, Result};
use rustls::{
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        WebPkiServerVerifier,
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use std::sync::Arc;

#[derive(Debug)]
struct SavedIdentity {
    certificate: CertificateDer<'static>,
    name: ServerName<'static>,
    standard: Arc<WebPkiServerVerifier>,
}
impl ServerCertVerifier for SavedIdentity {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _network_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() != self.certificate.as_ref() {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ));
        }
        self.standard
            .verify_server_cert(end_entity, intermediates, &self.name, ocsp, now)
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.standard
            .verify_tls12_signature(message, cert, signature)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.standard
            .verify_tls13_signature(message, cert, signature)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.standard.supported_verify_schemes()
    }
}
pub(super) fn configuration(der: &[u8], name: &str) -> Result<ClientConfig> {
    let invalid = || Error::Invalid("pinned TLS identity".into());
    let certificate = CertificateDer::from(der.to_vec());
    let name = ServerName::try_from(name.to_owned()).map_err(|_| invalid())?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = RootCertStore::empty();
    roots.add(certificate.clone()).map_err(|_| invalid())?;
    let standard = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|_| invalid())?;
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| invalid())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SavedIdentity {
            certificate,
            name,
            standard,
        }))
        .with_no_client_auth())
}
