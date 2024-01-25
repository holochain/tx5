use crate::*;

pub struct Imp {
    pub(crate) buf: tx5_go_pion::GoBuf,
    pub(crate) _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl std::fmt::Debug for Imp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buf").finish()
    }
}

impl Imp {
    pub(crate) fn from_raw(buf: tx5_go_pion::GoBuf) -> Self {
        Self {
            buf,
            _not_sync: std::marker::PhantomData,
        }
    }

    #[inline]
    pub fn from_slice<S: AsRef<[u8]>>(slice: S) -> Result<Self> {
        Ok(Self {
            buf: tx5_go_pion::GoBuf::from_slice(slice)?,
            _not_sync: std::marker::PhantomData,
        })
    }

    #[inline]
    pub fn from_json<S: serde::Serialize>(s: S) -> Result<Self> {
        let mut buf = tx5_go_pion::GoBuf::new()?;
        crate::deps::serde_json::to_writer(&mut buf, &s)?;
        Ok(Self {
            buf,
            _not_sync: std::marker::PhantomData,
        })
    }

    #[inline]
    #[allow(clippy::wrong_self_convention)] // ya, well, we need it mut
    pub fn len(&mut self) -> Result<usize> {
        self.buf.len()
    }
}

impl std::io::Read for Imp {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.buf.read(buf)
    }
}
