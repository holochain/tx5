use crate::*;
use parking_lot::Mutex;
use std::sync::Arc;
use tx5_go_pion_sys::API;

/// A precursor go pion webrtc DataChannel, awaiting an event handler.
#[derive(Debug)]
pub struct DataChannelSeed(pub(crate) usize, Arc<Mutex<Vec<DataChannelEvent>>>);

impl Drop for DataChannelSeed {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { API.data_chan_free(self.0) }
        }
    }
}

impl DataChannelSeed {
    pub(crate) fn new(data_chan_id: usize) -> Self {
        let hold = Arc::new(Mutex::new(Vec::new()));
        {
            let hold = hold.clone();
            register_data_chan_evt_cb(
                data_chan_id,
                Arc::new(move |evt| {
                    hold.lock().push(evt);
                }),
            );
        }
        Self(data_chan_id, hold)
    }

    /// Construct a real DataChannel by providing an event handler.
    pub fn handle<Cb>(mut self, cb: Cb) -> DataChannel
    where
        Cb: Fn(DataChannelEvent) + 'static + Send + Sync,
    {
        let cb: DataChanEvtCb = Arc::new(cb);
        let data_chan_id = self.0;
        self.0 = 0;
        replace_data_chan_evt_cb(data_chan_id, move || {
            for evt in self.1.lock().drain(..) {
                cb(evt);
            }
            cb
        });
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
    /// Get the label of this DataChannel.
    #[inline]
    pub fn label(&mut self) -> Result<GoBuf> {
        unsafe { Ok(GoBuf(API.data_chan_label(self.0)?)) }
    }

    /// Get the ready state of this DataChannel.
    #[inline]
    pub fn ready_state(&mut self) -> Result<usize> {
        unsafe { API.data_chan_ready_state(self.0) }
    }

    /// Set the buffered amount low threshold.
    /// Returns the current BufferedAmount.
    #[inline]
    pub fn set_buffered_amount_low_threshold(
        &mut self,
        threshold: usize,
    ) -> Result<usize> {
        unsafe {
            API.data_chan_set_buffered_amount_low_threshold(self.0, threshold)
        }
    }

    /// Send data to the remote peer on this DataChannel.
    /// Returns the current BufferedAmount.
    pub async fn send<'a, B>(&mut self, data: B) -> Result<usize>
    where
        B: Into<GoBufRef<'a>>,
    {
        // TODO - use OnBufferedAmountLow signal to implement backpressure
        let data_chan = self.0;
        r2id!(data);
        tokio::task::spawn_blocking(move || unsafe {
            API.data_chan_send(data_chan, data)
        })
        .await?
    }
}
