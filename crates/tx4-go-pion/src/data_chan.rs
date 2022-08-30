use crate::*;
use std::sync::Arc;
use tx4_go_pion_sys::API;

/// A precursor go pion webrtc DataChannel, awaiting an event handler.
#[derive(Debug)]
pub struct DataChannelSeed(pub(crate) usize);

impl Drop for DataChannelSeed {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { API.data_chan_free(self.0) }
        }
    }
}

impl DataChannelSeed {
    /// Construct a real DataChannel by providing an event handler.
    pub fn handle<Cb>(mut self, cb: Cb) -> DataChannel
    where
        Cb: Fn(DataChannelEvent) + 'static + Send + Sync,
    {
        let cb: DataChanEvtCb = Arc::new(cb);
        let data_chan_id = self.0;
        self.0 = 0;
        register_data_chan_evt_cb(data_chan_id, cb);
        DataChannel(data_chan_id)
    }
}

/// A go pion webrtc DataChannel.
#[derive(Debug)]
pub struct DataChannel(pub(crate) usize);

impl Drop for DataChannel {
    fn drop(&mut self) {
        unregister_data_chan_evt_cb(self.0);
        unsafe { API.data_chan_free(self.0) }
    }
}

impl DataChannel {
    /// Get the ready state of this DataChannel.
    #[inline]
    pub fn ready_state(&mut self) -> Result<usize> {
        unsafe { API.data_chan_ready_state(self.0) }
    }

    /// Send data to the remote peer on this DataChannel.
    #[inline]
    pub fn send(&mut self, data: GoBuf) -> Result<()> {
        unsafe { API.data_chan_send(self.0, data.0) }
    }
}
