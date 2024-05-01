use crate::*;

fn parse_url<R: AsRef<str>>(r: R) -> Result<(String, Option<PubKey>)> {
    let r = r.as_ref();
    let url = ::url::Url::parse(r).map_err(Error::other)?;
    match url.scheme() {
        "ws" | "wss" => (),
        scheme => {
            return Err(Error::other(format!(
                "invalid scheme, expected \"ws\" or \"wss\", got: {scheme:?}",
            )));
        }
    }

    if url.host_str().is_none() {
        return Err(Error::other("InvalidHost"));
    }

    let pub_key = match url.path_segments() {
        None => None,
        Some(mut it) => match it.next() {
            None => None,
            Some(v) => {
                if it.next().is_some() {
                    return Err(Error::other("ExtraPathInfo"));
                }
                use base64::Engine;
                let dec = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(v)
                    .map_err(Error::other)?;

                if dec.len() != 32 {
                    return Err(Error::other("InvalidPubKey"));
                }

                Some(PubKey(Arc::new(dec.try_into().unwrap())))
            }
        },
    };

    Ok((r.to_string(), pub_key))
}

/// A signal server url.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SigUrl(Arc<String>);

impl std::ops::Deref for SigUrl {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Debug for SigUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Display for SigUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for SigUrl {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl SigUrl {
    /// Parse a sig url.
    pub fn parse<R: AsRef<str>>(r: R) -> Result<Self> {
        let (url, pub_key) = parse_url(r)?;
        if pub_key.is_some() {
            return Err(Error::other("Expected sig url, got peer url"));
        }
        Ok(Self(Arc::new(url)))
    }

    /// Convert to a PeerUrl.
    pub fn to_peer(&self, pub_key: PubKey) -> PeerUrl {
        PeerUrl(Arc::new((format!("{}/{:?}", self.0, pub_key), pub_key)))
    }
}

/// A peer connection url.
#[derive(Clone)]
pub struct PeerUrl(Arc<(String, PubKey)>);

impl std::ops::Deref for PeerUrl {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0 .0
    }
}

impl std::cmp::PartialEq for PeerUrl {
    fn eq(&self, oth: &Self) -> bool {
        self.0 .0.eq(&oth.0 .0)
    }
}

impl std::cmp::Eq for PeerUrl {}

impl std::hash::Hash for PeerUrl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0 .0.hash(state);
    }
}

impl std::fmt::Debug for PeerUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0 .0.fmt(f)
    }
}

impl std::fmt::Display for PeerUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0 .0.fmt(f)
    }
}

impl AsRef<str> for PeerUrl {
    fn as_ref(&self) -> &str {
        self.0 .0.as_ref()
    }
}

impl PeerUrl {
    /// Parse a peer url.
    pub fn parse<R: AsRef<str>>(r: R) -> Result<Self> {
        let (url, pub_key) = parse_url(r)?;
        match pub_key {
            None => Err(Error::other("Expected peer url, got sig url")),
            Some(pub_key) => Ok(Self(Arc::new((url, pub_key)))),
        }
    }

    /// Get the pub key component of this url.
    pub fn pub_key(&self) -> &PubKey {
        &self.0 .1
    }

    /// Convert to a SigUrl.
    pub fn to_sig(&self) -> SigUrl {
        // already know this parses
        let url = ::url::Url::parse(&self.0 .0).unwrap();
        let fb_port = match url.scheme() {
            "ws" => 80,
            "wss" => 443,
            _ => unreachable!(),
        };
        let sig = format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str().unwrap(),
            url.port().unwrap_or(fb_port),
        );
        SigUrl(Arc::new(sig))
    }
}
