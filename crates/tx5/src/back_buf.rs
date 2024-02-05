//! Tx5 buffer abstractions.

use crate::*;

#[cfg(feature = "backend-go-pion")]
pub(crate) mod imp {
    mod imp_go_pion;
    pub use imp_go_pion::*;
}

/// Tx5 buffer type for sending and receiving data.
#[allow(clippy::len_without_is_empty)]
pub(crate) struct BackBuf {
    pub(crate) imp: imp::Imp,
    pub(crate) _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl std::fmt::Debug for BackBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.imp.fmt(f)
    }
}

impl BackBuf {
    pub(crate) fn from_raw(buf: tx5_go_pion::GoBuf) -> Self {
        Self {
            imp: imp::Imp::from_raw(buf),
            _not_sync: std::marker::PhantomData,
        }
    }

    /// Build a tx5 buffer from a slice.
    #[inline]
    pub fn from_slice<S: AsRef<[u8]>>(slice: S) -> Result<Self> {
        Ok(Self {
            imp: imp::Imp::from_slice(slice)?,
            _not_sync: std::marker::PhantomData,
        })
    }

    /// Serialize a type as json into a new BackBuf.
    #[inline]
    pub fn from_json<S: serde::Serialize>(s: S) -> Result<Self> {
        Ok(Self {
            imp: imp::Imp::from_json(s)?,
            _not_sync: std::marker::PhantomData,
        })
    }

    /// Get the length of this buffer.
    #[inline]
    pub fn len(&mut self) -> Result<usize> {
        self.imp.len()
    }
}

impl std::io::Read for BackBuf {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.imp.read(buf)
    }
}

/// Conversion type facilitating Into<&mut BackBuf>.
pub(crate) enum BackBufRef<'lt> {
    /// An owned BackBuf.
    Owned(Result<BackBuf>),

    /// A borrowed BackBuf.
    Borrowed(Result<&'lt mut BackBuf>),
}

impl From<BackBuf> for BackBufRef<'static> {
    fn from(b: BackBuf) -> Self {
        Self::Owned(Ok(b))
    }
}

impl<'lt> From<&'lt mut BackBuf> for BackBufRef<'lt> {
    fn from(b: &'lt mut BackBuf) -> Self {
        Self::Borrowed(Ok(b))
    }
}

impl<S: serde::Serialize> From<S> for BackBufRef<'static> {
    fn from(s: S) -> Self {
        Self::Owned(BackBuf::from_json(s))
    }
}
