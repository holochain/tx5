use crate::*;
use std::sync::Arc;

use ::url::Url;

/// Tx4 url.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tx4Url(Arc<Url>);

impl std::fmt::Debug for Tx4Url {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Display for Tx4Url {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for Tx4Url {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<Arc<Url>> for Tx4Url {
    fn as_ref(&self) -> &Arc<Url> {
        &self.0
    }
}

impl AsRef<Url> for Tx4Url {
    fn as_ref(&self) -> &Url {
        &*self.0
    }
}

impl Tx4Url {
    /// Parse a url from a str.
    pub fn new<R: AsRef<str>>(r: R) -> Result<Self> {
        let url = Url::parse(r.as_ref()).map_err(Error::err)?;

        match url.scheme() {
            "ws" | "wss" => (),
            scheme => {
                return Err(Error::err(format!(
                    "invalid scheme, expected \"ws\" or \"wss\", got: {:?}",
                    scheme,
                )));
            }
        }

        if url.host_str().is_none() {
            return Err(Error::id("InvalidHost"));
        }

        match url.path_segments() {
            // None is okay, it's a signal url
            None => (),
            Some(mut seg) => {
                // None on the first next is still okay
                if let Some(first) = seg.next() {
                    if first != "tx4-ws" {
                        return Err(Error::err(format!(
                            "invalid first path segment, expected \"tx4-ws\", got: {:?}",
                            first,
                        )));
                    }
                    match seg.next() {
                        None => return Err(Error::id("InvalidPubKey")),
                        Some(pk) => {
                            let _id = Id::from_b64(pk)?;
                        }
                    }
                }
            }
        }

        Ok(Self(Arc::new(url)))
    }

    /// If this url does not contain the `/tx4-ws/[pubkey]` path,
    /// then it references a signal server directly, and this function
    /// will return `true`. Otherwise, it is a client url.
    #[inline]
    pub fn is_server(&self) -> bool {
        !self.is_client()
    }

    /// If this url contains the `/tx4-ws/[pubkey]` path,
    /// then it is a client url, otherwise, it is a signal
    /// server url.
    pub fn is_client(&self) -> bool {
        match self.0.path_segments() {
            None => false,
            Some(mut seg) => match seg.next() {
                None => false,
                Some(_) => true,
            },
        }
    }

    /// Parse the "id" path segment of this url, if it is a client url.
    pub fn id(&self) -> Option<Id> {
        match self.0.path_segments() {
            None => None,
            Some(mut seg) => match seg.next() {
                None => None,
                Some(_) => match seg.next() {
                    None => None,
                    Some(id) => match Id::from_b64(id) {
                        Err(_) => None,
                        Ok(id) => Some(id),
                    },
                },
            },
        }
    }

    /// Parse the "host:port" portion of this url.
    pub fn endpoint(&self) -> String {
        let host = self.0.host_str().unwrap();
        let port = self.0.port().unwrap_or(443);
        format!("{}:{}", host, port)
    }
}
