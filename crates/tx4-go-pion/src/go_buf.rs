use crate::*;
use std::sync::Arc;
use tx4_go_pion_sys::API;

/// Interim step for conversion into a GoBuf.
pub struct IntoGoBuf(pub Result<GoBuf>);

impl IntoGoBuf {
    /// Serialize via serde_json into an IntoGoBuf.
    pub fn json<S: serde::Serialize>(s: S) -> Self {
        Self((|| {
            let mut b = GoBuf::new()?;
            serde_json::to_writer(&mut b, &s).map_err(Error::err)?;
            Ok(b)
        })())
    }
}

impl From<IntoGoBuf> for Result<GoBuf> {
    #[inline]
    fn from(i: IntoGoBuf) -> Self {
        i.0
    }
}

impl From<GoBuf> for IntoGoBuf {
    #[inline]
    fn from(b: GoBuf) -> Self {
        Self(Ok(b))
    }
}

impl<R: AsRef<[u8]>> From<R> for IntoGoBuf {
    #[inline]
    fn from(r: R) -> Self {
        Self(GoBuf::from_slice(r))
    }
}

/// A bytes.Buffer managed in go memory.
/// Rust can only access go memory safely during a callback.
#[derive(Debug)]
pub struct GoBuf(pub(crate) usize);

impl std::convert::TryFrom<&[u8]> for GoBuf {
    type Error = std::io::Error;

    #[inline]
    fn try_from(value: &[u8]) -> Result<Self> {
        GoBuf::from_slice(value)
    }
}

impl std::convert::TryFrom<&Vec<u8>> for GoBuf {
    type Error = std::io::Error;

    #[inline]
    fn try_from(value: &Vec<u8>) -> Result<Self> {
        GoBuf::from_slice(value)
    }
}

impl<const N: usize> std::convert::TryFrom<[u8; N]> for GoBuf {
    type Error = std::io::Error;

    #[inline]
    fn try_from(value: [u8; N]) -> Result<Self> {
        GoBuf::from_slice(value)
    }
}

impl std::convert::TryFrom<Vec<u8>> for GoBuf {
    type Error = std::io::Error;

    #[inline]
    fn try_from(value: Vec<u8>) -> Result<Self> {
        GoBuf::from_slice(value)
    }
}

impl std::convert::TryFrom<Box<[u8]>> for GoBuf {
    type Error = std::io::Error;

    #[inline]
    fn try_from(value: Box<[u8]>) -> Result<Self> {
        GoBuf::from_slice(value)
    }
}

impl std::convert::TryFrom<Arc<[u8]>> for GoBuf {
    type Error = std::io::Error;

    #[inline]
    fn try_from(value: Arc<[u8]>) -> Result<Self> {
        GoBuf::from_slice(value)
    }
}

impl Drop for GoBuf {
    fn drop(&mut self) {
        unsafe {
            API.buffer_free(self.0);
        }
    }
}

impl GoBuf {
    /// Construct a new bytes.Buffer in go memory.
    #[inline]
    pub fn new() -> Result<Self> {
        unsafe { Ok(Self(API.buffer_alloc()?)) }
    }

    /// Construct a new bytes.Buffer in go memory,
    /// copying in the provided slice.
    #[inline]
    pub fn from_slice<R: AsRef<[u8]>>(r: R) -> Result<Self> {
        let mut b = GoBuf::new()?;
        b.extend(r.as_ref())?;
        Ok(b)
    }

    /// Reserve additional capacity in this buffer.
    #[inline]
    pub fn reserve(&mut self, add: usize) -> Result<()> {
        unsafe { API.buffer_reserve(self.0, add) }
    }

    /// Extend this buffer with additional bytes.
    #[inline]
    pub fn extend(&mut self, add: &[u8]) -> Result<()> {
        unsafe { API.buffer_extend(self.0, add) }
    }

    /// Get access to the underlying buffer data.
    /// This data is allocated / managed by go, so it's only
    /// safe to access during a callback.
    #[inline]
    pub fn access<Cb, R>(&mut self, cb: Cb) -> Result<R>
    where
        Cb: FnOnce(Result<&mut [u8]>) -> Result<R>,
    {
        unsafe {
            match API.buffer_access(self.0, move |r| {
                match match r {
                    Ok((_id, data)) => cb(Ok(data)),
                    Err(e) => cb(Err(e)),
                } {
                    Ok(r) => Ok(r),
                    Err(e) => Err(e),
                }
            }) {
                Ok(r) => Ok(r),
                Err(e) => Err(e),
            }
        }
    }
}

impl std::io::Read for GoBuf {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        unsafe {
            API.buffer_read(self.0, buf.len(), move |r| match r {
                Ok(data) => {
                    let amt = data.len();
                    if amt == 1 {
                        buf[0] = data[0];
                    } else {
                        buf[..amt].copy_from_slice(data);
                    }
                    Ok(amt)
                }
                Err(err) => Err(err),
            })
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
        }
    }

    // TODO fill out like:
    // https://doc.rust-lang.org/src/std/io/impls.rs.html#231-300
}

impl std::io::Write for GoBuf {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.extend(buf).map_err(|err| {
            std::io::Error::new(std::io::ErrorKind::Other, err)
        })?;
        Ok(buf.len())
    }

    #[inline]
    fn write_vectored(
        &mut self,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::io::Result<usize> {
        let len = bufs.iter().map(|b| b.len()).sum();
        self.reserve(len).map_err(|err| {
            std::io::Error::new(std::io::ErrorKind::Other, err)
        })?;
        for buf in bufs {
            self.extend(buf).map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::Other, err)
            })?;
        }
        Ok(len)
    }

    /* unstable
    #[inline]
    fn is_write_vectored(&self) -> bool {
        true
    }
    */

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.extend(buf).map_err(|err| {
            std::io::Error::new(std::io::ErrorKind::Other, err)
        })?;
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buf_test() {
        let mut buf = GoBuf::new().unwrap();
        buf.reserve(5).unwrap();
        buf.extend(b"hello").unwrap();
        buf.access(|r| {
            assert_eq!(b"hello", r.unwrap());
            Ok(())
        })
        .unwrap();
    }
}
