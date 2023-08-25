//! Bootstrap Utilities.

use msgpackin_core::decode::{LenType, Token};
use msgpackin_core::num::Num;
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

    let res = HTTP
        .post(url.as_str())
        .body(body)
        .header("X-Op", op)
        .header(reqwest::header::CONTENT_TYPE, "application/octet")
        .send()
        .await
        .map_err(Error::err)?;

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
pub async fn boot_post(url: &str, entry: Vec<u8>) -> std::io::Result<()> {
    let _ = http(url, "put", entry).await?;
    Ok(())
}

/// Fetch entries from the bootstrap server.
pub async fn boot_fetch(
    url: &str,
    app_hash: &[u8; 32],
) -> std::io::Result<Vec<BootEntry>> {
    let mut enc = msgpackin_core::encode::Encoder::new();
    let mut out = Vec::new();

    out.extend_from_slice(&enc.enc_map_len(2));

    const SPACE: &[u8] = b"space";
    out.extend_from_slice(&enc.enc_str_len(SPACE.len() as u32));
    out.extend_from_slice(SPACE);

    let mut full_space = [0; 36];
    let loc = loc_bytes(app_hash);
    full_space[..32].copy_from_slice(app_hash);
    full_space[32..].copy_from_slice(&loc);
    out.extend_from_slice(&enc.enc_bin_len(full_space.len() as u32));
    out.extend_from_slice(&full_space);

    const LIMIT: &[u8] = b"limit";
    out.extend_from_slice(&enc.enc_str_len(LIMIT.len() as u32));
    out.extend_from_slice(LIMIT);

    out.extend_from_slice(&enc.enc_num(16));

    let res = http(url, "random", out).await?;

    let mut dec = msgpackin_core::decode::Decoder::new();

    let mut iter = dec.parse(&res);

    let len = match iter.next() {
        Some(Token::Len(LenType::Arr, len)) => len,
        _ => return Err(Error::id("ExpectedArrLen")),
    };

    let mut out = Vec::new();

    for _ in 0..len {
        let _b_len = match iter.next() {
            Some(Token::Len(LenType::Bin, b_len)) => b_len,
            _ => return Err(Error::id("ExpectedBinLen")),
        };

        let b = match iter.next() {
            Some(Token::Bin(v)) => v,
            _ => return Err(Error::id("ExpectedBin")),
        };

        out.push(BootEntry::decode(b)?);
    }

    Ok(out)
}

/// A bootstrap entry.
#[derive(Debug)]
pub struct BootEntry {
    /// public key
    pub pub_key: [u8; 32],

    /// application hash
    pub app_hash: [u8; 32],

    /// client url
    pub cli_url: String,

    /// signed timestamp
    pub signed_at: std::time::SystemTime,

    /// expires timestamp
    pub expires_at: std::time::SystemTime,

    /// application data
    pub data: Vec<u8>,
}

impl BootEntry {
    /// Encode a bootstrap entry.
    pub fn encode(&self) -> Vec<u8> {
        let mut enc = msgpackin_core::encode::Encoder::new();
        let mut out = Vec::new();

        out.extend_from_slice(&enc.enc_map_len(6));

        const SPACE: &[u8] = b"space";
        out.extend_from_slice(&enc.enc_str_len(SPACE.len() as u32));
        out.extend_from_slice(SPACE);

        let mut full_space = [0; 36];
        let loc = loc_bytes(&self.app_hash);
        full_space[..32].copy_from_slice(&self.app_hash);
        full_space[32..].copy_from_slice(&loc);
        out.extend_from_slice(&enc.enc_bin_len(full_space.len() as u32));
        out.extend_from_slice(&full_space);

        const AGENT: &[u8] = b"agent";
        out.extend_from_slice(&enc.enc_str_len(AGENT.len() as u32));
        out.extend_from_slice(AGENT);

        let mut full_agent = [0; 36];
        let loc = loc_bytes(&self.pub_key);
        full_agent[..32].copy_from_slice(&self.pub_key);
        full_agent[32..].copy_from_slice(&loc);
        out.extend_from_slice(&enc.enc_bin_len(full_agent.len() as u32));
        out.extend_from_slice(&full_agent);

        const URLS: &[u8] = b"urls";
        out.extend_from_slice(&enc.enc_str_len(URLS.len() as u32));
        out.extend_from_slice(URLS);

        out.extend_from_slice(&enc.enc_arr_len(1));

        out.extend_from_slice(&enc.enc_str_len(self.cli_url.len() as u32));
        out.extend_from_slice(self.cli_url.as_bytes());

        const SIGNED_AT_MS: &[u8] = b"signed_at_ms";
        out.extend_from_slice(&enc.enc_str_len(SIGNED_AT_MS.len() as u32));
        out.extend_from_slice(SIGNED_AT_MS);

        let sa = self
            .signed_at
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        out.extend_from_slice(&enc.enc_num(sa));

        const EXPIRES_AFTER_MS: &[u8] = b"expires_after_ms";
        out.extend_from_slice(&enc.enc_str_len(EXPIRES_AFTER_MS.len() as u32));
        out.extend_from_slice(EXPIRES_AFTER_MS);

        let ea = self
            .expires_at
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            - sa;
        out.extend_from_slice(&enc.enc_num(ea));

        const META_INFO: &[u8] = b"meta_info";
        out.extend_from_slice(&enc.enc_str_len(META_INFO.len() as u32));
        out.extend_from_slice(META_INFO);

        out.extend_from_slice(&enc.enc_bin_len(self.data.len() as u32));
        out.extend_from_slice(&self.data);

        out
    }

    /// Sign an encoded bootstrap entry.
    pub fn sign(&self, encoded: &[u8], signature: &[u8; 64]) -> Vec<u8> {
        let mut enc = msgpackin_core::encode::Encoder::new();
        let mut out = Vec::new();

        out.extend_from_slice(&enc.enc_map_len(3));

        const AGENT: &[u8] = b"agent";
        out.extend_from_slice(&enc.enc_str_len(AGENT.len() as u32));
        out.extend_from_slice(AGENT);

        let mut full_agent = [0; 36];
        let loc = loc_bytes(&self.pub_key);
        full_agent[..32].copy_from_slice(&self.pub_key);
        full_agent[32..].copy_from_slice(&loc);
        out.extend_from_slice(&enc.enc_bin_len(full_agent.len() as u32));
        out.extend_from_slice(&full_agent);

        const SIGNATURE: &[u8] = b"signature";
        out.extend_from_slice(&enc.enc_str_len(SIGNATURE.len() as u32));
        out.extend_from_slice(SIGNATURE);

        out.extend_from_slice(&enc.enc_bin_len(signature.len() as u32));
        out.extend_from_slice(&signature[..]);

        const AGENT_INFO: &[u8] = b"agent_info";
        out.extend_from_slice(&enc.enc_str_len(AGENT_INFO.len() as u32));
        out.extend_from_slice(AGENT_INFO);

        out.extend_from_slice(&enc.enc_bin_len(encoded.len() as u32));
        out.extend_from_slice(encoded);

        out
    }

    /// Decode a bootstrap entry.
    pub fn decode(encoded: &[u8]) -> std::io::Result<Self> {
        let mut sig_pub_key: &[u8] = &[];
        let mut sig: &[u8] = &[];
        let mut inner: &[u8] = &[];

        {
            let mut dec_signed = msgpackin_core::decode::Decoder::new();

            let mut dec_iter = dec_signed.parse(encoded);

            let len = match dec_iter.next() {
                Some(Token::Len(LenType::Map, len)) => len,
                _ => return Err(Error::id("ExpectedMapLen")),
            };

            for _ in 0..len {
                let _k_len = match dec_iter.next() {
                    Some(Token::Len(LenType::Str, k_len)) => k_len,
                    _ => return Err(Error::id("ExpectedStrLen")),
                };

                let k = match dec_iter.next() {
                    Some(Token::Bin(k)) => k,
                    _ => return Err(Error::id("ExpectedBin")),
                };

                let _v_len = match dec_iter.next() {
                    Some(Token::Len(LenType::Bin, v_len)) => v_len,
                    _ => return Err(Error::id("ExpectedBinLen")),
                };

                let v = match dec_iter.next() {
                    Some(Token::Bin(v)) => v,
                    _ => return Err(Error::id("ExpectedBin")),
                };

                match k {
                    b"agent" => sig_pub_key = v,
                    b"signature" => sig = v,
                    b"agent_info" => inner = v,
                    oth => {
                        return Err(Error::err(format!(
                            "UnexpectedToken: {}",
                            String::from_utf8_lossy(oth)
                        )))
                    }
                }
            }
        }

        if sig_pub_key.is_empty() {
            return Err(Error::id("MissingPubKey"));
        }

        if sig.is_empty() {
            return Err(Error::id("MissingSignature"));
        }

        // TODO - validate signature

        let mut dec = msgpackin_core::decode::Decoder::new();

        let mut dec_iter = dec.parse(inner);

        let len = match dec_iter.next() {
            Some(Token::Len(LenType::Map, len)) => len,
            _ => return Err(Error::id("ExpectedMapLen")),
        };

        let mut pub_key: &[u8] = &[];
        let mut app_hash: &[u8] = &[];
        let mut cli_url: &[u8] = &[];
        let mut signed_at = 0;
        let mut expires_at = 0;
        let mut data: &[u8] = &[];

        for _ in 0..len {
            let _k_len = match dec_iter.next() {
                Some(Token::Len(LenType::Str, k_len)) => k_len,
                _ => return Err(Error::id("ExpectedStrLen")),
            };

            let k = match dec_iter.next() {
                Some(Token::Bin(k)) => k,
                _ => return Err(Error::id("ExpectedBin")),
            };

            match k {
                b"space" => {
                    let _v_len = match dec_iter.next() {
                        Some(Token::Len(LenType::Bin, v_len)) => v_len,
                        _ => return Err(Error::id("ExpectedBinLen")),
                    };

                    app_hash = match dec_iter.next() {
                        Some(Token::Bin(v)) => v,
                        _ => return Err(Error::id("ExpectedBin")),
                    };
                }
                b"agent" => {
                    let _v_len = match dec_iter.next() {
                        Some(Token::Len(LenType::Bin, v_len)) => v_len,
                        _ => return Err(Error::id("ExpectedBinLen")),
                    };

                    pub_key = match dec_iter.next() {
                        Some(Token::Bin(v)) => v,
                        _ => return Err(Error::id("ExpectedBin")),
                    };
                }
                b"urls" => {
                    let a_len = match dec_iter.next() {
                        Some(Token::Len(LenType::Arr, a_len)) => a_len,
                        _ => return Err(Error::id("ExpectedArrLen")),
                    };

                    for i in 0..a_len {
                        let _u_len = match dec_iter.next() {
                            Some(Token::Len(LenType::Str, u_len)) => u_len,
                            _ => return Err(Error::id("ExpectedStrLen")),
                        };

                        let u = match dec_iter.next() {
                            Some(Token::Bin(u)) => u,
                            _ => return Err(Error::id("ExpectedBin")),
                        };

                        if i == 0 {
                            cli_url = u;
                        }
                    }
                }
                b"signed_at_ms" => {
                    signed_at = match dec_iter.next() {
                        Some(Token::Num(Num::Unsigned(n))) => n,
                        _ => return Err(Error::id("ExpectedNum")),
                    };
                }
                b"expires_after_ms" => {
                    expires_at = match dec_iter.next() {
                        Some(Token::Num(Num::Unsigned(n))) => n,
                        _ => return Err(Error::id("ExpectedNum")),
                    };
                }
                b"meta_info" => {
                    let _v_len = match dec_iter.next() {
                        Some(Token::Len(LenType::Bin, v_len)) => v_len,
                        _ => return Err(Error::id("ExpectedBinLen")),
                    };

                    data = match dec_iter.next() {
                        Some(Token::Bin(v)) => v,
                        _ => return Err(Error::id("ExpectedBin")),
                    };
                }
                oth => {
                    return Err(Error::err(format!(
                        "UnexpectedToken: {}",
                        String::from_utf8_lossy(oth)
                    )))
                }
            }
        }

        let mut pk = [0; 32];
        pk.copy_from_slice(&pub_key[..32]);

        let mut ah = [0; 32];
        ah.copy_from_slice(&app_hash[..32]);

        let sa = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_millis(signed_at);
        let ea = sa + std::time::Duration::from_millis(expires_at);

        Ok(BootEntry {
            pub_key: pk,
            app_hash: ah,
            cli_url: String::from_utf8_lossy(cli_url).to_string(),
            signed_at: sa,
            expires_at: ea,
            data: data.to_vec(),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const TEST_ENCODED: &str = "g6VhZ2VudMQkAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgIgST39qXNpZ25hdHVyZcRAAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDA6phZ2VudF9pbmZvxPGGpXNwYWNlxCQBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAX7Pzr6lYWdlbnTEJAICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICIEk9/aR1cmxzkdlGd3NzOi8vdHg1LmhvbG8uaG9zdC90eDUtd3MvWVdSMERYVEprV21wZnM3TFlYUEJNWEFHR0NlWHhCQ0xPR0V1V2FFMVl2VaxzaWduZWRfYXRfbXMqsGV4cGlyZXNfYWZ0ZXJfbXMbqW1ldGFfaW5mb8QegbtkaHRfc3RvcmFnZV9hcmNfaGFsZl9sZW5ndGgq";

    #[test]
    fn boot_entry_encode_decode() {
        let enc = base64::decode(TEST_ENCODED).unwrap();
        let entry = BootEntry::decode(&enc).unwrap();
        //println!("{entry:#?}");

        let re_enc_info = entry.encode();

        /*
        let mut dec = msgpackin_core::decode::Decoder::new();
        for token in dec.parse(&re_enc_info) {
            println!("{token:?}");
        }
        */

        let re_enc = entry.sign(&re_enc_info, &[0x03; 64]);

        /*
        let mut dec = msgpackin_core::decode::Decoder::new();
        for token in dec.parse(&re_enc) {
            println!("{token:?}");
        }
        */

        assert_eq!(re_enc, enc);
    }

    #[tokio::test]
    #[ignore = "don't run these tests against the live production server"]
    async fn boot() {
        const URL: &str = "https://bootstrap.holo.host";

        boot_ping(URL).await.unwrap();

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
            cli_url: "wss://testing".to_string(),
            signed_at: std::time::SystemTime::now(),
            expires_at: std::time::SystemTime::now()
                + std::time::Duration::from_secs(60) * 20,
            data: b"hello".to_vec(),
        };

        let enc1 = entry.encode();

        let sig: sodoken::BufWriteSized<{ sodoken::sign::BYTES }> =
            sodoken::BufWriteSized::new_no_lock();
        sodoken::sign::detached(sig.clone(), enc1.clone(), sk)
            .await
            .unwrap();
        let sig = sig.to_read_sized();

        let mut signature = [0; 64];
        signature.copy_from_slice(&*sig.read_lock_sized());

        let enc2 = entry.sign(&enc1, &signature);

        boot_post(URL, enc2).await.unwrap();

        // wait for cloudflare consistency
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        println!("{:#?}", boot_fetch(URL, &[1; 32]).await.unwrap());
    }
}
