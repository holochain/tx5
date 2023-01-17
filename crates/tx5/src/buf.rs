//! Tx5 buffer abstractions.

use crate::*;

#[cfg(feature = "backend-go-pion")]
pub(crate) mod imp {
    mod imp_go_pion;
    pub use imp_go_pion::*;
}

/// Tx5 buffer creation type via std::io::Write.
pub struct BufWriter {
    imp: imp::ImpWriter,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl BufWriter {
    /// Create a new Tx5 buffer writer.
    #[inline]
    pub fn new() -> Result<Self> {
        Ok(Self {
            imp: imp::ImpWriter::new()?,
            _not_sync: std::marker::PhantomData,
        })
    }

    /// Indicate we are done writing, and extract the internal buffer.
    #[inline]
    pub fn finish(self) -> Buf {
        Buf {
            imp: self.imp.finish(),
            _not_sync: std::marker::PhantomData,
        }
    }
}

impl std::io::Write for BufWriter {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.imp.write(buf)
    }

    #[inline]
    fn write_vectored(
        &mut self,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::io::Result<usize> {
        self.imp.write_vectored(bufs)
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.imp.write_all(buf)
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        self.imp.flush()
    }
}

/// Tx5 buffer type for sending and receiving data.
#[allow(clippy::len_without_is_empty)]
pub struct Buf {
    pub(crate) imp: imp::Imp,
    pub(crate) _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl std::fmt::Debug for Buf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.imp.fmt(f)
    }
}

impl Buf {
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

    /// Build a tx5 buffer using std::io::Write.
    #[inline]
    pub fn from_writer() -> Result<BufWriter> {
        BufWriter::new()
    }

    /// Serialize a type as json into a new Buf.
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

    /// Attempt to clone this buffer.
    #[inline]
    pub fn try_clone(&mut self) -> Result<Self> {
        Ok(Self {
            imp: self.imp.try_clone()?,
            _not_sync: std::marker::PhantomData,
        })
    }

    /// Copy the buffer out into a rust `Vec<u8>`.
    #[inline]
    pub fn to_vec(&mut self) -> Result<Vec<u8>> {
        self.imp.to_vec()
    }

    /// Deserialize this buffer as json bytes
    /// into a type implementing serde::DeserializeOwned.
    #[inline]
    pub fn to_json<D>(&mut self) -> Result<D>
    where
        D: serde::de::DeserializeOwned + Sized,
    {
        self.imp.to_json()
    }
}

impl std::io::Read for Buf {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.imp.read(buf)
    }
}

/// Conversion type facilitating Into<&mut Buf>.
pub enum BufRef<'lt> {
    /// An owned Buf.
    Owned(Result<Buf>),

    /// A borrowed Buf.
    Borrowed(Result<&'lt mut Buf>),
}

impl<'lt> BufRef<'lt> {
    /// Get a mutable reference to the buffer.
    pub fn as_mut_ref(&'lt mut self) -> Result<&'lt mut Buf> {
        match self {
            BufRef::Owned(o) => match o {
                Ok(o) => Ok(o),
                Err(e) => Err(e.err_clone()),
            },
            BufRef::Borrowed(b) => match b {
                Ok(b) => Ok(b),
                Err(e) => Err(e.err_clone()),
            },
        }
    }
}

impl From<Buf> for BufRef<'static> {
    fn from(b: Buf) -> Self {
        Self::Owned(Ok(b))
    }
}

impl<'lt> From<&'lt mut Buf> for BufRef<'lt> {
    fn from(b: &'lt mut Buf) -> Self {
        Self::Borrowed(Ok(b))
    }
}

impl<S: serde::Serialize> From<S> for BufRef<'static> {
    fn from(s: S) -> Self {
        Self::Owned(Buf::from_json(s))
    }
}
