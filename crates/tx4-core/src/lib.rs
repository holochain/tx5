#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]

//! Holochain WebRTC P2P Communication Ecosystem Core Types.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

/// Tx4 core error type
pub struct Error {
    /// Error name / identifier
    pub name: String,

    /// Additional error information
    pub info: String,
}

impl From<String> for Error {
    #[inline]
    fn from(s: String) -> Self {
        Self {
            name: s,
            info: String::default(),
        }
    }
}

impl From<&str> for Error {
    #[inline]
    fn from(s: &str) -> Self {
        s.to_string().into()
    }
}

impl From<Error> for std::io::Error {
    #[inline]
    fn from(e: Error) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, e)
    }
}

impl Error {
    /// Construct a new Tx4 core error instance
    pub fn new<N>(n: N) -> Self
    where
        N: Into<String>,
    {
        Self {
            name: n.into(),
            info: String::default(),
        }
    }

    /// Construct a new Tx4 core error instance
    pub fn new_info<N, I>(n: N, i: I) -> Self
    where
        N: Into<String>,
        I: Into<String>,
    {
        Self {
            name: n.into(),
            info: i.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)?;
        if !self.info.is_empty() {
            f.write_str(": ")?;
            f.write_str(&self.info)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

impl std::error::Error for Error {}

/// Extension trait to extract a name from a Tx4 core error type
pub trait ErrorExt {
    /// Get the "name" identifier of this error type,
    /// or the string representation
    fn name(&self) -> std::borrow::Cow<'_, str>;
}

impl ErrorExt for Error {
    #[inline]
    fn name(&self) -> std::borrow::Cow<'_, str> {
        (&self.name).into()
    }
}

impl ErrorExt for std::io::Error {
    #[inline]
    fn name(&self) -> std::borrow::Cow<'_, str> {
        match self.get_ref() {
            Some(r) => match r.downcast_ref::<Error>() {
                Some(r) => (&r.name).into(),
                None => r.to_string().into(),
            },
            None => self.to_string().into(),
        }
    }
}

/// Tx4 core result type
pub type Result<T> = std::result::Result<T, std::io::Error>;
