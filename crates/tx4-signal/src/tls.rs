//! TLS encryption types.

use crate::*;
use once_cell::sync::Lazy;
use sha2::Digest;
use std::sync::Arc;
use tokio_rustls::*;

/// The well-known CA keypair in plaintext pem format.
/// Some TLS clients require CA roots to validate client-side certificates.
/// By publishing the private keys here, we are essentially allowing
/// self-signed client certificates.
const WK_CA_KEYPAIR_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgkxOEyiRyocjLRpQk
RE7/bOwmHtkdLLGQrlz23m4aKQOhRANCAATUDekPM40vfqOMxf00KZwRk6gSciHx
xkzPZovign1qmbu0vZstKoVLXoGvlA/Kral9txqhSEGqIL7TdbKyMMQz
-----END PRIVATE KEY-----"#;

/// The well-known pseudo name/id for the well-known lair CA root.
const WK_CA_ID: &str = "aKdjnmYOn1HVc_RwSdxR6qa.aQLW3d5D1nYiSSO2cOrcT7a";

/// This doesn't need to be pub... We need the rcgen::Certificate
/// with the private keys still integrated in order to sign certs.
static WK_CA_RCGEN_CERT: Lazy<Arc<rcgen::Certificate>> = Lazy::new(|| {
    let mut params = rcgen::CertificateParams::new(vec![WK_CA_ID.into()]);
    params.alg = &rcgen::PKCS_ECDSA_P256_SHA256;
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .extended_key_usages
        .push(rcgen::ExtendedKeyUsagePurpose::Any);
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        "Lair Well-Known Pseudo-Self-Signing CA",
    );
    params
        .distinguished_name
        .push(rcgen::DnType::OrganizationName, "Holochain Foundation");
    params.key_pair = Some(rcgen::KeyPair::from_pem(WK_CA_KEYPAIR_PEM).unwrap());
    let cert = rcgen::Certificate::from_params(params).unwrap();
    Arc::new(cert)
});

/// The well-known lair CA pseudo-self-signing certificate.
static WK_CA_CERT_DER: Lazy<Arc<Vec<u8>>> = Lazy::new(|| {
    let cert = WK_CA_RCGEN_CERT.as_ref();
    let cert = cert.serialize_der().unwrap();
    Arc::new(cert)
});

/// DER encoded tls certificate
pub struct TlsCertDer(pub Box<[u8]>);

/// DER encoded tls private key
pub struct TlsPkDer(pub Box<[u8]>);

/// Generate a new der-encoded tls certificate and private key
pub fn gen_tls_cert_pair() -> Result<(TlsCertDer, TlsPkDer)> {
    let mut r = [0; 32];
    use ring::rand::SecureRandom;
    ring::rand::SystemRandom::new()
        .fill(&mut r[..])
        .map_err(|_| Error::id("SystemRandomFailure"))?;

    let sni = format!(
        "rtc{}ctr",
        base64::encode_config(r, base64::URL_SAFE_NO_PAD)
    );

    let mut params = rcgen::CertificateParams::new(vec![sni.clone()]);
    params.alg = &rcgen::PKCS_ECDSA_P256_SHA256;
    params
        .extended_key_usages
        .push(rcgen::ExtendedKeyUsagePurpose::Any);
    params
        .extended_key_usages
        .push(rcgen::ExtendedKeyUsagePurpose::ServerAuth);
    params
        .extended_key_usages
        .push(rcgen::ExtendedKeyUsagePurpose::ClientAuth);
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        format!("Lair Pseudo-Self-Signed Cert {}", &sni),
    );

    let cert = rcgen::Certificate::from_params(params).map_err(Error::err)?;

    let priv_key = cert.serialize_private_key_der();

    let root_cert = &**WK_CA_RCGEN_CERT;
    let cert_der = cert
        .serialize_der_with_signer(root_cert)
        .map_err(Error::err)?;

    Ok((
        TlsCertDer(cert_der.into_boxed_slice()),
        TlsPkDer(priv_key.into_boxed_slice()),
    ))
}

/// Single shared keylog file all sessions can report to
static KEY_LOG: Lazy<Arc<dyn rustls::KeyLog>> = Lazy::new(|| Arc::new(rustls::KeyLogFile::new()));

/// TLS configuration builder
pub struct TlsConfigBuilder {
    alpn: Vec<Vec<u8>>,
    cert: Option<(TlsCertDer, TlsPkDer)>,
    cipher_suites: Vec<rustls::SupportedCipherSuite>,
    protocol_versions: Vec<&'static rustls::SupportedProtocolVersion>,
    key_log: bool,
    session_storage: usize,
}

impl Default for TlsConfigBuilder {
    fn default() -> Self {
        Self {
            alpn: Vec::new(),
            cert: None,
            cipher_suites: vec![
                rustls::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
                rustls::cipher_suite::TLS13_AES_256_GCM_SHA384,
            ],
            protocol_versions: vec![&rustls::version::TLS13],
            key_log: false,
            session_storage: 512,
        }
    }
}

impl TlsConfigBuilder {
    /// Push an alpn protocol to use with this connection. If none are
    /// set, alpn negotiotion will not happen. Otherwise, specify the
    /// most preferred protocol first
    pub fn with_alpn(mut self, alpn: &[u8]) -> Self {
        self.alpn.push(alpn.into());
        self
    }

    /// Set the certificate / private key to use with this TLS config.
    /// If not set, a new certificate and private key will be generated
    pub fn with_cert(mut self, cert: TlsCertDer, pk: TlsPkDer) -> Self {
        self.cert = Some((cert, pk));
        self
    }

    /// Enable or disable TLS keylogging. Note, even if enabled,
    /// the standard `SSLKEYLOGFILE` environment variable must
    /// be present for keys to be written.
    pub fn with_keylog(mut self, key_log: bool) -> Self {
        self.key_log = key_log;
        self
    }

    /// Finalize/build a TlsConfig from this builder
    pub fn build(self) -> Result<TlsConfig> {
        let (cert, pk) = match self.cert {
            Some(r) => r,
            None => gen_tls_cert_pair()?,
        };

        let mut digest = sha2::Sha256::new();
        digest.update(&cert.0);
        let digest = Arc::new(Id(digest.finalize().into()));

        let cert = rustls::Certificate(cert.0.into_vec());
        let pk = rustls::PrivateKey(pk.0.into_vec());

        let root_cert = rustls::Certificate(WK_CA_CERT_DER.to_vec());
        let mut root_store = rustls::RootCertStore::empty();
        root_store.add(&root_cert).map_err(Error::err)?;

        let mut srv = rustls::ServerConfig::builder()
            .with_cipher_suites(self.cipher_suites.as_slice())
            .with_safe_default_kx_groups()
            .with_protocol_versions(self.protocol_versions.as_slice())
            .map_err(Error::err)?
            .with_client_cert_verifier(rustls::server::NoClientAuth::new())
            .with_single_cert(vec![cert], pk)
            .map_err(Error::err)?;

        if self.key_log {
            srv.key_log = KEY_LOG.clone();
        }
        srv.ticketer = rustls::Ticketer::new().map_err(Error::err)?;
        srv.session_storage = rustls::server::ServerSessionMemoryCache::new(self.session_storage);
        for alpn in self.alpn.iter() {
            srv.alpn_protocols.push(alpn.clone());
        }

        let mut cli = rustls::ClientConfig::builder()
            .with_cipher_suites(self.cipher_suites.as_slice())
            .with_safe_default_kx_groups()
            .with_protocol_versions(self.protocol_versions.as_slice())
            .map_err(Error::err)?
            .with_custom_certificate_verifier(Arc::new(V))
            .with_no_client_auth();

        if self.key_log {
            cli.key_log = KEY_LOG.clone();
        }
        cli.session_storage = rustls::client::ClientSessionMemoryCache::new(self.session_storage);
        for alpn in self.alpn.iter() {
            cli.alpn_protocols.push(alpn.clone());
        }

        Ok(TlsConfig {
            srv: Arc::new(srv),
            cli: Arc::new(cli),
            digest,
        })
    }
}

/// A fully configured tls p2p session state instance
#[derive(Clone)]
pub struct TlsConfig {
    pub(crate) srv: Arc<rustls::ServerConfig>,
    pub(crate) cli: Arc<rustls::ClientConfig>,
    digest: Arc<Id>,
}

impl std::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConfig")
            .field("cert_digest", &self.digest)
            .finish()
    }
}

impl TlsConfig {
    /// Builder for generating TlsConfig instances
    pub fn builder() -> TlsConfigBuilder {
        TlsConfigBuilder::default()
    }

    /// Get the sha256 hash of the TLS certificate representing this server
    pub fn cert_digest(&self) -> &Arc<Id> {
        &self.digest
    }
}

struct V;

impl rustls::client::ServerCertVerifier for V {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::Certificate,
        intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        now: std::time::SystemTime,
    ) -> std::result::Result<rustls::client::ServerCertVerified, rustls::Error> {
        priv_verify_server_cert(end_entity, intermediates, now)
            .map_err(|err| rustls::Error::General(format!("{:?}", err)))
    }
}

fn priv_verify_server_cert(
    end_entity: &rustls::Certificate,
    intermediates: &[rustls::Certificate],
    now: std::time::SystemTime,
) -> Result<rustls::client::ServerCertVerified> {
    let (cert, chain, trustroots) = prepare(end_entity, intermediates)?;
    let now = webpki::Time::try_from(now)
        .map_err(|_| rustls::Error::FailedToGetCurrentTime)
        .map_err(Error::err)?;

    // Treat server certs like client certs
    cert.verify_is_valid_tls_client_cert(
        SUPPORTED_SIG_ALGS,
        &webpki::TlsClientTrustAnchors(&trustroots),
        &chain,
        now,
    )
    .map_err(Error::err)
    .map(|_| rustls::client::ServerCertVerified::assertion())
}

static SUPPORTED_SIG_ALGS: &[&webpki::SignatureAlgorithm] = &[
    &webpki::ECDSA_P256_SHA256,
    &webpki::ECDSA_P256_SHA384,
    &webpki::ECDSA_P384_SHA256,
    &webpki::ECDSA_P384_SHA384,
    &webpki::ED25519,
    &webpki::RSA_PSS_2048_8192_SHA256_LEGACY_KEY,
    &webpki::RSA_PSS_2048_8192_SHA384_LEGACY_KEY,
    &webpki::RSA_PSS_2048_8192_SHA512_LEGACY_KEY,
    &webpki::RSA_PKCS1_2048_8192_SHA256,
    &webpki::RSA_PKCS1_2048_8192_SHA384,
    &webpki::RSA_PKCS1_2048_8192_SHA512,
    &webpki::RSA_PKCS1_3072_8192_SHA384,
];

type CertChainAndRoots<'a, 'b> = (
    webpki::EndEntityCert<'a>,
    Vec<&'a [u8]>,
    Vec<webpki::TrustAnchor<'b>>,
);

fn prepare<'a, 'b>(
    end_entity: &'a rustls::Certificate,
    intermediates: &'a [rustls::Certificate],
) -> Result<CertChainAndRoots<'a, 'b>> {
    // EE cert must appear first.
    let cert = webpki::EndEntityCert::try_from(end_entity.0.as_ref()).map_err(Error::err)?;

    let intermediates: Vec<&'a [u8]> = intermediates.iter().map(|cert| cert.0.as_ref()).collect();

    let trustroots =
        vec![webpki::TrustAnchor::try_from_cert_der(&*WK_CA_CERT_DER).map_err(Error::err)?];

    Ok((cert, intermediates, trustroots))
}
