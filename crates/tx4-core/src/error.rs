/// Tx4 core error type.
#[derive(Clone)]
pub struct Error {
    /// Error identifier.
    pub id: String,

    /// Additional error information.
    pub info: String,
}

impl From<()> for Error {
    #[inline]
    fn from(_: ()) -> Self {
        Self {
            id: "Error".into(),
            info: String::default(),
        }
    }
}

impl From<String> for Error {
    #[inline]
    fn from(id: String) -> Self {
        Self {
            id,
            info: String::default(),
        }
    }
}

impl From<&str> for Error {
    #[inline]
    fn from(id: &str) -> Self {
        id.to_string().into()
    }
}

impl From<Error> for std::io::Error {
    #[inline]
    fn from(e: Error) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, e)
    }
}

impl From<std::io::Error> for Error {
    #[inline]
    fn from(e: std::io::Error) -> Self {
        let info = e.to_string();
        if let Some(e) = e.into_inner() {
            if let Ok(e) = e.downcast::<Error>() {
                return *e;
            }
        }
        Self {
            id: "Error".into(),
            info,
        }
    }
}

impl From<&std::io::Error> for Error {
    #[inline]
    fn from(e: &std::io::Error) -> Self {
        if let Some(r) = e.get_ref() {
            if let Some(r) = r.downcast_ref::<Error>() {
                return r.clone();
            }
        }
        Self {
            id: "Error".into(),
            info: e.to_string(),
        }
    }
}

impl Error {
    /// Construct a new Tx4 core error instance with input as an identifier.
    pub fn id<T>(t: T) -> std::io::Error
    where
        T: Into<String>,
    {
        Self {
            id: t.into(),
            info: String::default(),
        }
        .into()
    }

    /// Construct a new Tx4 core error instance with input as additional info.
    pub fn err<E>(e: E) -> std::io::Error
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        Self {
            id: "Error".into(),
            info: e.into().to_string(),
        }
        .into()
    }

    /// Construct a new Tx4 core error instance with input as additional info.
    pub fn str<S>(s: S) -> std::io::Error
    where
        S: Into<String>,
    {
        Self {
            id: "Error".into(),
            info: s.into(),
        }
        .into()
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.id)?;
        if !self.info.is_empty() {
            f.write_str(": ")?;
            f.write_str(&self.info)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for Error {}

/// Extension trait to extract a name from a Tx4 core error type.
pub trait ErrorExt {
    /// Get the identifier of this error type,
    /// or the string representation.
    fn id(&self) -> std::borrow::Cow<'_, str>;

    /// Clone the error maintaining any meta info is available
    /// if we are a tx4 error type.
    fn err_clone(&self) -> std::io::Error;
}

impl ErrorExt for Error {
    #[inline]
    fn id(&self) -> std::borrow::Cow<'_, str> {
        (&self.id).into()
    }

    #[inline]
    fn err_clone(&self) -> std::io::Error {
        self.clone().into()
    }
}

impl ErrorExt for std::io::Error {
    #[inline]
    fn id(&self) -> std::borrow::Cow<'_, str> {
        match self.get_ref() {
            Some(r) => match r.downcast_ref::<Error>() {
                Some(r) => (&r.id).into(),
                None => r.to_string().into(),
            },
            None => self.to_string().into(),
        }
    }

    #[inline]
    fn err_clone(&self) -> std::io::Error {
        Error::from(self).into()
    }
}

#[doc(inline)]
pub use std::io::Result;
