//! Tx4 data channel abstractions.

use crate::*;

/// Events emitted by a DataChannel.
pub enum DataChannelEvent {
    /// The DataChannel is closed.
    Close,

    /// An incoming message on the DataChannel.
    Message(Buf),
}

#[cfg(feature = "backend-go-pion")]
pub(crate) mod imp {
    mod imp_go_pion;
    pub use imp_go_pion::*;
}

/// Tx4 data channel seed.
pub struct DataChannelSeed {
    pub(crate) imp: imp::ImpChanSeed,
    pub(crate) _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl DataChannelSeed {
    /// Convert this DataChannelSeed into a true DataChannel
    /// by providing an event handler and awaiting ready state.
    pub async fn handle<Cb>(self, cb: Cb) -> Result<DataChannel>
    where
        Cb: Fn(DataChannelEvent) + 'static + Send + Sync,
    {
        let DataChannelSeed { imp, .. } = self;
        let imp = imp.handle(cb).await?;
        Ok(DataChannel {
            imp,
            _not_sync: std::marker::PhantomData,
        })
    }
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
