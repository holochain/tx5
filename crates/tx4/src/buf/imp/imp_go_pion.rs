use crate::*;

pub struct ImpWriter {
    buf: tx4_go_pion::GoBuf,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl ImpWriter {
    #[inline]
    pub fn new() -> Result<Self> {
        Ok(Self {
            buf: tx4_go_pion::GoBuf::new()?,
            _not_sync: std::marker::PhantomData,
        })
    }

    #[inline]
    pub fn finish(self) -> Imp {
        Imp {
            buf: self.buf,
            _not_sync: std::marker::PhantomData,
        }
    }
}

impl std::io::Write for ImpWriter {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.write(buf)
    }

    #[inline]
    fn write_vectored(
        &mut self,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::io::Result<usize> {
        self.buf.write_vectored(bufs)
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.buf.write_all(buf)
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        self.buf.flush()
    }
}

pub struct Imp {
    pub(crate) buf: tx4_go_pion::GoBuf,
    pub(crate) _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl Imp {
    #[inline]
    pub fn from_slice<S: AsRef<[u8]>>(slice: S) -> Result<Self> {
        Ok(Self {
            buf: tx4_go_pion::GoBuf::from_slice(slice)?,
            _not_sync: std::marker::PhantomData,
        })
    }

    #[inline]
    pub fn from_json<S: serde::Serialize>(s: S) -> Result<Self> {
        let mut buf = tx4_go_pion::GoBuf::new()?;
        crate::deps::serde_json::to_writer(&mut buf, &s)?;
        Ok(Self {
            buf,
            _not_sync: std::marker::PhantomData,
        })
    }

    #[inline]
    #[allow(clippy::wrong_self_convention)] // ya, well, we need it mut
    pub fn to_vec(&mut self) -> Result<Vec<u8>> {
        self.buf.to_vec()
    }

    #[inline]
    #[allow(clippy::wrong_self_convention)] // ya, well, we need it mut
    pub fn to_json<D>(&mut self) -> Result<D>
    where
        D: serde::de::DeserializeOwned + Sized,
    {
        self.buf.as_json()
    }
}

impl std::io::Read for Imp {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.buf.read(buf)
    }
}
