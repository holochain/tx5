use crate::*;
use bytes::{Bytes, BytesMut, Buf};

pub type Raw = Bytes;

pub struct ImpWriter {
    buf: BytesMut,
}

impl ImpWriter {
    #[inline]
    pub fn new() -> Result<Self> {
        Ok(Self {
            buf: BytesMut::new(),
        })
    }

    #[inline]
    pub fn finish(self) -> Imp {
        Imp { buf: self.buf.freeze() }
    }
}

impl std::io::Write for ImpWriter {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    #[inline]
    fn write_vectored(
        &mut self,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::io::Result<usize> {
        let mut total_written = 0;
        for buf in bufs {
            self.write_all(buf)?;
            total_written += buf.len();
        }
        Ok(total_written)
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.buf.extend_from_slice(buf);
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct Imp {
    pub(crate) buf: Bytes,
}

impl std::fmt::Debug for Imp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buf").finish()
    }
}

impl Imp {
    pub(crate) fn from_raw(buf: Bytes) -> Self {
        let slice = buf.as_ref();
        let mut bytes_mut = BytesMut::with_capacity(slice.len());
        bytes_mut.extend_from_slice(slice);
        Self { buf: bytes_mut.freeze() }
    }

    #[inline]
    pub fn from_slice<S: AsRef<[u8]>>(slice: S) -> Result<Self> {
        let slicearr = slice.as_ref();
        let mut bytes_mut = BytesMut::with_capacity(slicearr.len());
        bytes_mut.extend_from_slice(slicearr);
        Ok(Self { buf: bytes_mut.freeze() })
    }

    #[inline]
    pub fn from_json<S: serde::Serialize>(s: S) -> Result<Self> {
        let mut impwriter = ImpWriter::new()?;
        crate::deps::serde_json::to_writer(&mut impwriter, &s)?;
        Ok(impwriter.finish())
    }

    #[inline]
    pub fn try_clone(&self) -> Result<Self> {
        Ok(Self {
            buf: self.buf.clone(),
        })
    }

    #[inline]
    pub fn len(&self) -> Result<usize> {
        Ok(self.buf.len())
    }

    #[inline]
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        Ok(self.buf.to_vec())
    }

    #[inline]
    pub fn to_json<D>(&self) -> Result<D>
    where
        D: serde::de::DeserializeOwned + Sized,
    {
        let bytes = &self.buf;
        serde_json::from_slice(bytes.as_ref()).map_err(Error::err)
    }
}

impl std::io::Read for Imp {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut reader = self.buf.as_ref().reader();
        reader.read(buf)
    }
}
