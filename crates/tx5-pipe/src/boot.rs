//! Bootstrap Utilities.

use tx5_core::Error;

fn loc_bytes(data: &[u8; 32]) -> [u8; 4] {
    let hash = blake2b_simd::Params::new()
        .hash_length(16)
        .hash(data)
        .as_bytes()
        .to_vec();
    let mut out = [hash[0], hash[1], hash[2], hash[3]];
    for i in (4..16).step_by(4) {
        out[0] ^= hash[i];
        out[1] ^= hash[i + 1];
        out[2] ^= hash[i + 2];
        out[3] ^= hash[i + 3];
    }
    out
}

static HTTP: once_cell::sync::Lazy<reqwest::Client> =
    once_cell::sync::Lazy::new(reqwest::Client::new);

async fn http(url: &str, op: &str, body: Vec<u8>) -> std::io::Result<Vec<u8>> {
    let url = format!("{url}?net=tx5");

    let req = HTTP
        .post(url.as_str())
        .body(body)
        .header("X-Op", op)
        .header(reqwest::header::CONTENT_TYPE, "application/octet");

    tracing::trace!(?req);

    let res = req.send().await.map_err(Error::err)?;

    if res.status().is_success() {
        Ok(res.bytes().await.map_err(Error::err)?.to_vec())
    } else {
        Err(Error::err(res.text().await.map_err(Error::err)?))
    }
}

/// Ping the bootstrap server.
pub async fn boot_ping(url: &str) -> std::io::Result<()> {
    let _ = http(url, "now", Vec::new()).await?;
    Ok(())
}

/// Post an encoded entry to the bootstrap server.
pub async fn boot_put(url: &str, entry: Vec<u8>) -> std::io::Result<()> {
    let _ = http(url, "put", entry).await?;
    Ok(())
}

/// Fetch entries from the bootstrap server.
pub async fn boot_random(
    url: &str,
    app_hash: &[u8; 32],
) -> std::io::Result<Vec<BootEntry>> {
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    #[serde(bound(deserialize = "'a: 'de"))]
    struct Query<'a> {
        #[serde(with = "serde_bytes")]
        pub space: &'a [u8],
        pub limit: u32,
    }

    let mut full_space = [0; 36];
    let loc = loc_bytes(app_hash);
    full_space[..32].copy_from_slice(app_hash);
    full_space[32..].copy_from_slice(&loc);

    let query = msgpackin::to_bytes(&Query {
        space: &full_space,
        limit: 16,
    })
    .map_err(Error::err)?;

    let res = http(url, "random", query).await?;

    #[derive(Debug, serde::Deserialize)]
    struct Good<'a>(#[serde(with = "serde_bytes")] &'a [u8]);

    #[derive(Debug, serde::Deserialize)]
    struct Bad(Vec<u8>);

    let mut out = Vec::new();

    let tmp: Result<Vec<Good>, _> = msgpackin::from_ref(&res);

    match tmp {
        Ok(list) => {
            for item in list {
                out.push(BootEntry::decode(item.0)?);
            }
        }
        Err(_) => {
            let tmp: Vec<Bad> =
                msgpackin::from_ref(&res).map_err(Error::err)?;
            for raw in tmp {
                out.push(BootEntry::decode(&raw.0)?);
            }
        }
    }

    Ok(out)
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct MetaEncode {
    pub dht_storage_arc_half_length: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "'a: 'de"))]
struct AgentInfoEncode<'a> {
    #[serde(with = "serde_bytes")]
    pub space: &'a [u8],
    #[serde(with = "serde_bytes")]
    pub agent: &'a [u8],
    pub urls: Vec<msgpackin::value::Utf8StrRef<'a>>,
    pub signed_at_ms: u64,
    // WARNING-this is a weird offset from the signed_at_ms time!!!!
    pub expires_after_ms: u64,
    #[serde(with = "serde_bytes")]
    pub meta_info: &'a [u8],
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct AgentInfoSignedEncode<'a> {
    #[serde(with = "serde_bytes")]
    pub agent: &'a [u8],
    #[serde(with = "serde_bytes")]
    pub signature: &'a [u8],
    #[serde(with = "serde_bytes")]
    pub agent_info: &'a [u8],
}

/// A bootstrap entry.
#[derive(Debug)]
pub struct BootEntry {
    /// public key
    pub pub_key: [u8; 32],

    /// application hash
    pub app_hash: [u8; 32],

    /// client url
    pub cli_url: Option<String>,

    /// signed timestamp
    pub signed_at: std::time::SystemTime,

    /// expires timestamp
    pub expires_at: std::time::SystemTime,

    /// application data
    pub data: Vec<u8>,
}

impl BootEntry {
    /// Encode a bootstrap entry.
    pub fn encode(&self) -> std::io::Result<Vec<u8>> {
        let mut full_space = [0; 36];
        let loc = loc_bytes(&self.app_hash);
        full_space[..32].copy_from_slice(&self.app_hash);
        full_space[32..].copy_from_slice(&loc);

        let mut full_agent = [0; 36];
        let loc = loc_bytes(&self.pub_key);
        full_agent[..32].copy_from_slice(&self.pub_key);
        full_agent[32..].copy_from_slice(&loc);

        let mut urls = Vec::new();

        if let Some(url) = &self.cli_url {
            urls.push(msgpackin::value::Utf8StrRef(url.as_bytes()));
        }

        let signed_at_ms = self
            .signed_at
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let expires_after_ms = self
            .expires_at
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            - signed_at_ms;

        let mut meta_info = msgpackin::to_bytes(&MetaEncode {
            dht_storage_arc_half_length: self.data.len() as u32,
        })
        .map_err(Error::err)?;

        meta_info.extend_from_slice(&self.data);

        msgpackin::to_bytes(&AgentInfoEncode {
            space: &full_space[..],
            agent: &full_agent[..],
            urls,
            signed_at_ms,
            expires_after_ms,
            meta_info: &meta_info,
        })
        .map_err(Error::err)
    }

    /// Sign an encoded bootstrap entry.
    pub fn sign(
        &self,
        encoded: &[u8],
        signature: &[u8; 64],
    ) -> std::io::Result<Vec<u8>> {
        let mut full_agent = [0; 36];
        let loc = loc_bytes(&self.pub_key);
        full_agent[..32].copy_from_slice(&self.pub_key);
        full_agent[32..].copy_from_slice(&loc);

        msgpackin::to_bytes(&AgentInfoSignedEncode {
            agent: &full_agent[..],
            signature: &signature[..],
            agent_info: encoded,
        })
        .map_err(Error::err)
    }

    /// Decode a bootstrap entry.
    pub fn decode(encoded: &[u8]) -> std::io::Result<Self> {
        let raw_sig: AgentInfoSignedEncode =
            msgpackin::from_ref(encoded).map_err(Error::err)?;
        let raw_info: AgentInfoEncode =
            msgpackin::from_ref(raw_sig.agent_info).map_err(Error::err)?;
        let raw_meta: MetaEncode =
            msgpackin::from_ref(raw_info.meta_info).map_err(Error::err)?;
        let meta_len = std::cmp::min(
            raw_meta.dht_storage_arc_half_length as usize,
            raw_info.meta_info.len(),
        );
        let meta = &raw_info.meta_info[(raw_info.meta_info.len() - meta_len)..];

        if raw_sig.agent != raw_info.agent || raw_sig.agent.len() != 36 {
            return Err(Error::id("InvalidPubKey"));
        }

        if raw_info.space.len() != 36 {
            return Err(Error::id("InvalidAppHash"));
        }

        let mut pub_key = [0; 32];
        pub_key.copy_from_slice(&raw_info.agent[..32]);

        let mut app_hash = [0; 32];
        app_hash.copy_from_slice(&raw_info.space[..32]);

        let cli_url = if raw_info.urls.is_empty() {
            None
        } else {
            raw_info.urls[0]
                .as_str()
                .map(|m| Some(m.to_string()))
                .unwrap_or(None)
        };

        let signed_at = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_millis(raw_info.signed_at_ms);
        let expires_at = signed_at
            + std::time::Duration::from_millis(raw_info.expires_after_ms);

        // TODO - validate signature

        Ok(BootEntry {
            pub_key,
            app_hash,
            cli_url,
            signed_at,
            expires_at,
            data: meta.to_vec(),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn init_tracing() {
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::filter::EnvFilter::from_default_env(),
            )
            .with_file(true)
            .with_line_number(true)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    const TEST_ENCODED: &str = "g6VhZ2VudMQkAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgIgST39qXNpZ25hdHVyZcRAAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDA6phZ2VudF9pbmZvxP2GpXNwYWNlxCQBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAX7Pzr6lYWdlbnTEJAICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICIEk9/aR1cmxzkdlGd3NzOi8vdHg1LmhvbG8uaG9zdC90eDUtd3MvWVdSMERYVEprV21wZnM3TFlYUEJNWEFHR0NlWHhCQ0xPR0V1V2FFMVl2VaxzaWduZWRfYXRfbXMqsGV4cGlyZXNfYWZ0ZXJfbXMbqW1ldGFfaW5mb8QqgbtkaHRfc3RvcmFnZV9hcmNfaGFsZl9sZW5ndGgMaGVsbG8gd29ybGQh";

    #[test]
    fn boot_entry_encode_decode() {
        init_tracing();

        let enc = base64::decode(TEST_ENCODED).unwrap();
        let entry = BootEntry::decode(&enc).unwrap();
        //println!("{entry:#?}");

        let re_enc_info = entry.encode().unwrap();

        let re_enc = entry.sign(&re_enc_info, &[0x03; 64]).unwrap();

        //println!("{}", base64::encode(&re_enc));

        assert_eq!(re_enc, enc);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bootstrap_sanity() {
        init_tracing();

        let (driver, addr, shutdown) =
            kitsune_p2p_bootstrap::run(([127, 0, 0, 1], 0), vec![])
                .await
                .unwrap();
        tokio::task::spawn(driver);

        struct OnDrop(Option<kitsune_p2p_bootstrap::BootstrapShutdown>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                if let Some(s) = self.0.take() {
                    s();
                }
            }
        }
        let _on_drop = OnDrop(Some(shutdown));

        let url = format!("http://127.0.0.1:{}", addr.port());

        println!("url: {url}");

        boot_ping(&url).await.unwrap();

        let pk: sodoken::BufWriteSized<{ sodoken::sign::PUBLICKEYBYTES }> =
            sodoken::BufWriteSized::new_no_lock();
        let sk: sodoken::BufWriteSized<{ sodoken::sign::SECRETKEYBYTES }> =
            sodoken::BufWriteSized::new_mem_locked().unwrap();
        sodoken::sign::keypair(pk.clone(), sk.clone())
            .await
            .unwrap();
        let pk = pk.to_read_sized();
        let sk = sk.to_read_sized();

        let mut pub_key = [0; 32];
        pub_key.copy_from_slice(&*pk.read_lock_sized());

        let entry = BootEntry {
            pub_key,
            app_hash: [1; 32],
            cli_url: Some("wss://testing".to_string()),
            signed_at: std::time::SystemTime::now(),
            expires_at: std::time::SystemTime::now()
                + std::time::Duration::from_secs(60) * 20,
            data: b"hello".to_vec(),
        };

        let enc1 = entry.encode().unwrap();

        let sig: sodoken::BufWriteSized<{ sodoken::sign::BYTES }> =
            sodoken::BufWriteSized::new_no_lock();
        sodoken::sign::detached(sig.clone(), enc1.clone(), sk)
            .await
            .unwrap();
        let sig = sig.to_read_sized();

        let mut signature = [0; 64];
        signature.copy_from_slice(&*sig.read_lock_sized());

        let enc2 = entry.sign(&enc1, &signature).unwrap();

        boot_put(&url, enc2).await.unwrap();

        // wait for cloudflare consistency
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        println!("{:#?}", boot_random(&url, &[1; 32]).await.unwrap());
    }
}
