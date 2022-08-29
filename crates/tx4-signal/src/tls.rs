//! TLS encryption types.

use crate::*;
use once_cell::sync::Lazy;
use sha2::Digest;
use std::sync::Arc;
use tokio_rustls::*;

/// PEM Encoded Data
pub struct Pem(pub String);

impl serde::Serialize for Pem {
    fn serialize<S>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(None)?;
        for line in self.0.split("\r\n") {
            if !line.is_empty() {
                seq.serialize_element(line)?;
            }
        }
        seq.end()
    }
}

impl<'de> serde::Deserialize<'de> for Pem {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let tmp: Vec<&'de str> = serde::Deserialize::deserialize(deserializer)?;
        Ok(Pem(tmp.join("\r\n")))
    }
}

impl Pem {
    /// Parse the Pem data, returning DER encoded bytes.
    pub fn to_der(&self) -> Result<Vec<u8>> {
        let mut cursor = std::io::Cursor::new(self.0.as_bytes());
        match rustls_pemfile::read_one(&mut cursor)?
            .ok_or_else(|| Error::id("InvalidPEM"))?
        {
            rustls_pemfile::Item::X509Certificate(d) => Ok(d),
            rustls_pemfile::Item::RSAKey(d) => Ok(d),
            rustls_pemfile::Item::PKCS8Key(d) => Ok(d),
            rustls_pemfile::Item::ECKey(d) => Ok(d),
            _ => Err(Error::id("InvalidPEM")),
        }
    }
}

/// Typedef for a PEM encoded TLS certificate.
pub type TlsCertPem = Pem;

/// Typedef for a PEM encoded TLS private key.
pub type TlsPrivKeyPem = Pem;

/// Generate a new self-signed TLS certificate and private key for dev testing.
pub fn tls_self_signed() -> Result<(TlsCertPem, TlsPrivKeyPem)> {
    let mut r = [0; 32];
    use ring::rand::SecureRandom;
    ring::rand::SystemRandom::new()
        .fill(&mut r[..])
        .map_err(|_| Error::id("SystemRandomFailure"))?;

    let sni = format!(
        "tx4{}4xt",
        base64::encode_config(r, base64::URL_SAFE_NO_PAD)
    );
    let mut params = rcgen::CertificateParams::new(vec![sni]);
    params.alg = &rcgen::PKCS_ECDSA_P256_SHA256;
    let cert = rcgen::Certificate::from_params(params).map_err(Error::err)?;

    let pk = Pem(cert.serialize_private_key_pem());
    let cert = Pem(cert.serialize_pem().map_err(Error::err)?);

    Ok((cert, pk))
}

/// Single shared keylog file all sessions can report to
static KEY_LOG: Lazy<Arc<dyn rustls::KeyLog>> =
    Lazy::new(|| Arc::new(rustls::KeyLogFile::new()));

static NATIVE_CERTS: Lazy<
    std::result::Result<Vec<rustls::Certificate>, String>,
> = Lazy::new(|| {
    rustls_native_certs::load_native_certs()
        .map(|certs| {
            certs
                .into_iter()
                .map(|cert| rustls::Certificate(cert.0))
                .collect()
        })
        .map_err(|err| err.to_string())
});

/// TLS configuration builder
pub struct TlsConfigBuilder {
    alpn: Vec<Vec<u8>>,
    cert: Option<(TlsCertPem, TlsPrivKeyPem)>,
    cipher_suites: Vec<rustls::SupportedCipherSuite>,
    protocol_versions: Vec<&'static rustls::SupportedProtocolVersion>,
    key_log: bool,
    session_storage: usize,
    danger_no_server_verify: bool,
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
            danger_no_server_verify: false,
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
    pub fn with_cert(mut self, cert: TlsCertPem, pk: TlsPrivKeyPem) -> Self {
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

    /// Disable server TLS certificate verification
    /// if you want to dev test with a self-signed certificate.
    pub fn with_danger_no_server_verify(
        mut self,
        danger_no_server_verify: bool,
    ) -> Self {
        self.danger_no_server_verify = danger_no_server_verify;
        self
    }

    /// Finalize/build a TlsConfig from this builder
    pub fn build(self) -> Result<TlsConfig> {
        let (cert, pk) = match self.cert {
            Some(r) => r,
            None => tls_self_signed()?,
        };

        let mut digest = sha2::Sha256::new();
        digest.update(&cert.0);
        let digest = Id(digest.finalize().into());

        let cert = rustls::Certificate(cert.to_der()?);
        let pk = rustls::PrivateKey(pk.to_der()?);

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
        srv.session_storage =
            rustls::server::ServerSessionMemoryCache::new(self.session_storage);
        for alpn in self.alpn.iter() {
            srv.alpn_protocols.push(alpn.clone());
        }

        let cli = rustls::ClientConfig::builder()
            .with_cipher_suites(self.cipher_suites.as_slice())
            .with_safe_default_kx_groups()
            .with_protocol_versions(self.protocol_versions.as_slice())
            .map_err(Error::err)?;

        let mut cli = if self.danger_no_server_verify {
            cli.with_custom_certificate_verifier(Arc::new(V))
                .with_no_client_auth()
        } else {
            let mut roots = rustls::RootCertStore::empty();
            for cert in (*NATIVE_CERTS).as_ref().map_err(Error::str)?.iter() {
                roots.add(cert).map_err(Error::err)?;
            }
            cli.with_root_certificates(roots).with_no_client_auth()
        };

        if self.key_log {
            cli.key_log = KEY_LOG.clone();
        }
        cli.session_storage =
            rustls::client::ClientSessionMemoryCache::new(self.session_storage);
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
    #[allow(dead_code)]
    pub(crate) srv: Arc<rustls::ServerConfig>,
    #[allow(dead_code)]
    pub(crate) cli: Arc<rustls::ClientConfig>,
    digest: Id,
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
    pub fn cert_digest(&self) -> &Id {
        &self.digest
    }
}

struct V;

impl rustls::client::ServerCertVerifier for V {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> std::result::Result<rustls::client::ServerCertVerified, rustls::Error>
    {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}
