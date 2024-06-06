use crate::*;

struct UrlCanon {
    pub canon: String,
    pub pub_key: Option<PubKey>,
}

impl std::ops::Deref for UrlCanon {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.canon
    }
}

impl std::cmp::PartialEq for UrlCanon {
    fn eq(&self, oth: &Self) -> bool {
        self.canon.eq(&oth.canon)
    }
}

impl std::cmp::Eq for UrlCanon {}

impl std::hash::Hash for UrlCanon {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.canon.hash(state);
    }
}

impl std::fmt::Debug for UrlCanon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.canon.fmt(f)
    }
}

impl std::fmt::Display for UrlCanon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.canon.fmt(f)
    }
}

impl serde::Serialize for UrlCanon {
    fn serialize<S>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.canon.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for UrlCanon {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let url: &'de str = serde::Deserialize::deserialize(deserializer)?;
        UrlCanon::parse_url(url).map_err(serde::de::Error::custom)
    }
}

fn scheme_port(u: &::url::Url) -> Option<(&'static str, u16)> {
    match u.scheme() {
        "ws" => Some(("ws", u.port().unwrap_or(80))),
        "wss" => Some(("wss", u.port().unwrap_or(443))),
        _ => None,
    }
}

impl UrlCanon {
    fn parse_url<R: AsRef<str>>(r: R) -> Result<Self> {
        let r = r.as_ref();
        let url = ::url::Url::parse(r).map_err(Error::other)?;

        let (scheme, port) = match scheme_port(&url) {
            Some((scheme, port)) => (scheme, port),
            None => {
                return Err(Error::other(format!(
                    "invalid scheme, expected \"ws\" or \"wss\", got: {:?}",
                    url.scheme(),
                )));
            }
        };

        let host = match url.host_str() {
            None => return Err(Error::other("InvalidHost")),
            Some(host) => host,
        };

        let pub_key = match url.path_segments() {
            None => None,
            Some(mut it) => match it.next() {
                None => None,
                Some(v) => {
                    if it.next().is_some() {
                        return Err(Error::other("ExtraPathInfo"));
                    }
                    if v.is_empty() {
                        None
                    } else {
                        use base64::Engine;
                        let dec =
                            base64::engine::general_purpose::URL_SAFE_NO_PAD
                                .decode(v)
                                .map_err(Error::other)?;

                        if dec.len() != 32 {
                            return Err(Error::other("InvalidPubKey"));
                        }

                        Some(PubKey(Arc::new(dec.try_into().unwrap())))
                    }
                }
            },
        };

        let canon = match &pub_key {
            Some(pub_key) => format!("{scheme}://{host}:{port}/{pub_key:?}"),
            None => format!("{scheme}://{host}:{port}"),
        };

        Ok(UrlCanon { canon, pub_key })
    }
}

/// A signal server url.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct SigUrl(Arc<UrlCanon>);

impl std::ops::Deref for SigUrl {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for SigUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for SigUrl {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl SigUrl {
    /// Parse a sig url.
    pub fn parse<R: AsRef<str>>(r: R) -> Result<Self> {
        let canon = UrlCanon::parse_url(r)?;
        if canon.pub_key.is_some() {
            return Err(Error::other("Expected sig url, got peer url"));
        }
        Ok(Self(Arc::new(canon)))
    }

    /// Convert to a PeerUrl.
    pub fn to_peer(&self, pub_key: PubKey) -> PeerUrl {
        let canon = UrlCanon {
            canon: format!("{}/{:?}", self.0, pub_key),
            pub_key: Some(pub_key),
        };
        PeerUrl(Arc::new(canon))
    }
}

/// A peer connection url.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct PeerUrl(Arc<UrlCanon>);

impl std::ops::Deref for PeerUrl {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for PeerUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for PeerUrl {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl PeerUrl {
    /// Parse a peer url.
    pub fn parse<R: AsRef<str>>(r: R) -> Result<Self> {
        let canon = UrlCanon::parse_url(r)?;
        if canon.pub_key.is_none() {
            return Err(Error::other("Expected peer url, got sig url"));
        }
        Ok(Self(Arc::new(canon)))
    }

    /// Get the pub key component of this url.
    pub fn pub_key(&self) -> &PubKey {
        self.0.pub_key.as_ref().unwrap()
    }

    /// Convert to a SigUrl.
    pub fn to_sig(&self) -> SigUrl {
        // already know this parses
        let url = ::url::Url::parse(&self.0).unwrap();
        let (scheme, port) = scheme_port(&url).unwrap();
        let canon =
            format!("{}://{}:{}", scheme, url.host_str().unwrap(), port);
        SigUrl(Arc::new(UrlCanon {
            canon,
            pub_key: None,
        }))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn sig_peer_convertable() {
        let sig = SigUrl::parse("ws://a.b:80").unwrap();
        assert_eq!("ws://a.b:80", sig.as_ref());
        let peer = sig.to_peer(PubKey(Arc::new([0; 32])));
        assert_eq!(
            "ws://a.b:80/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            peer.as_ref(),
        );
        let sig2 = peer.to_sig();
        assert_eq!(sig, sig2);
    }

    #[test]
    fn canon_wss_port() {
        assert_eq!(
            "wss://a.b:443",
            SigUrl::parse("wss://a.b").unwrap().as_ref()
        );
        assert_eq!(
            "wss://a.b:443/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            PeerUrl::parse(
                "wss://a.b/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            )
            .unwrap()
            .as_ref()
        );
    }

    #[test]
    fn canon_ws_port() {
        assert_eq!("ws://a.b:80", SigUrl::parse("ws://a.b").unwrap().as_ref());
        assert_eq!(
            "ws://a.b:80/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            PeerUrl::parse(
                "ws://a.b/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            )
            .unwrap()
            .as_ref()
        );
    }

    #[test]
    fn display() {
        assert_eq!(
            "ws://a.b:80".to_string(),
            SigUrl::parse("ws://a.b:80").unwrap().to_string(),
        );
        assert_eq!(
            "ws://a.b:80/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            PeerUrl::parse(
                "ws://a.b:80/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            )
            .unwrap()
            .to_string()
        );
    }

    #[test]
    fn serialize_sig() {
        let orig = SigUrl::parse("ws://a.b:80").unwrap();
        let s = serde_json::to_string(&orig).unwrap();
        assert_eq!("\"ws://a.b:80\"", s.as_str());
        let s: SigUrl = serde_json::from_str(&s).unwrap();
        assert_eq!(orig, s);
    }

    #[test]
    fn serialize_peer() {
        let orig = PeerUrl::parse(
            "ws://a.b:80/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap();
        let s = serde_json::to_string(&orig).unwrap();
        assert_eq!(
            "\"ws://a.b:80/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"",
            s.as_str()
        );
        let s: PeerUrl = serde_json::from_str(&s).unwrap();
        assert_eq!(orig, s);
    }

    #[test]
    fn pub_key() {
        let p = PeerUrl::parse(
            "ws://a.b:80/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap();
        assert_eq!(&PubKey(Arc::new([0; 32])), p.pub_key());
    }
}
