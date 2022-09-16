//! Tx4 data channel abstractions.

use crate::*;

#[cfg(feature = "backend-go-pion")]
mod imp {
    mod imp_go_pion;
    pub use imp_go_pion::*;
}

/// Tx4 data channel.
pub struct DataChannel {
    imp: imp::ImpChan,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl DataChannel {
    /// Get the label associated with this DataChannel.
    pub fn label(&mut self) -> Result<Vec<u8>> {
        self.imp.label()
    }

    /// Returns `true` if this DataChannel is closed.
    /// Further calls to `send` will result in error.
    pub fn is_closed(&mut self) -> Result<bool> {
        self.imp.is_closed()
    }

    /// Send data to the remote end of this DataChannel.
    pub async fn send<'a, B>(&mut self, data: B) -> Result<()>
    where
        B: Into<&'a mut Buf>,
    {
        self.imp.send(data).await
    }
}
