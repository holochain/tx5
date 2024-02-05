use crate::*;
use std::sync::{Arc, Mutex, Weak};
use tx5_go_pion_sys::API;

pub(crate) struct DataChanCore {
    data_chan_id: usize,
    evt_send: tokio::sync::mpsc::UnboundedSender<DataChannelEvent>,
    drop_err: Error,
}

impl Drop for DataChanCore {
    fn drop(&mut self) {
        let _ = self
            .evt_send
            .send(DataChannelEvent::Error(self.drop_err.clone()));
        unregister_data_chan(self.data_chan_id);
        unsafe {
            API.data_chan_free(self.data_chan_id);
        }
    }
}

impl DataChanCore {
    pub fn new(
        data_chan_id: usize,
        evt_send: tokio::sync::mpsc::UnboundedSender<DataChannelEvent>,
    ) -> Self {
        Self {
            data_chan_id,
            evt_send,
            drop_err: Error::id("DataChannelDropped").into(),
        }
    }

    pub fn close(&mut self, err: Error) {
        // self.evt_send.send_err() is called in Drop impl
        self.drop_err = err;
    }
}

#[derive(Clone)]
pub(crate) struct WeakDataChan(
    pub(crate) Weak<Mutex<std::result::Result<DataChanCore, Error>>>,
);

macro_rules! data_chan_strong_core {
    ($inner:expr, $ident:ident, $block:block) => {
        match &mut *$inner.lock().unwrap() {
            Ok($ident) => $block,
            Err(err) => Result::Err(err.clone().into()),
        }
    };
}

macro_rules! data_chan_weak_core {
    ($inner:expr, $ident:ident, $block:block) => {
        match $inner.upgrade() {
            Some(strong) => data_chan_strong_core!(strong, $ident, $block),
            None => Result::Err(Error::id("DataChannelClosed")),
        }
    };
}

impl WeakDataChan {
    pub fn send_evt(&self, evt: DataChannelEvent) -> Result<()> {
        data_chan_weak_core!(self.0, core, {
            core.evt_send
                .send(evt)
                .map_err(|_| Error::id("DataChannelClosed"))
        })
    }
}

/// A go pion webrtc DataChannel.
pub struct DataChannel(Arc<Mutex<std::result::Result<DataChanCore, Error>>>);

impl std::fmt::Debug for DataChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let res = match &*self.0.lock().unwrap() {
            Ok(core) => format!("DataChannel(open, {})", core.data_chan_id),
            Err(err) => format!("DataChannel(closed, {:?})", err),
        };
        f.write_str(&res)
    }
}

impl DataChannel {
    pub(crate) fn new(
        data_chan_id: usize,
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<DataChannelEvent>) {
        let (evt_send, evt_recv) = tokio::sync::mpsc::unbounded_channel();

        let strong = Arc::new(Mutex::new(Ok(DataChanCore::new(
            data_chan_id,
            evt_send.clone(),
        ))));

        let weak = WeakDataChan(Arc::downgrade(&strong));

        register_data_chan(data_chan_id, weak);

        // we might have missed some state callbacks
        if let Ok(ready_state) =
            unsafe { API.data_chan_ready_state(data_chan_id) }
        {
            // this is a terrible suggestion clippy
            #[allow(clippy::comparison_chain)]
            if ready_state == 2 {
                let _ = evt_send.send(DataChannelEvent::Open);
            } else if ready_state > 2 {
                let _ = evt_send.send(DataChannelEvent::Close);
            }
        }

        (Self(strong), evt_recv)
    }

    /// Close this data channel.
    pub fn close(&self, err: Error) {
        let mut tmp = Err(err.clone());

        {
            let mut lock = self.0.lock().unwrap();
            let mut do_swap = false;
            if let Ok(core) = &mut *lock {
                core.close(err);
                do_swap = true;
            }
            if do_swap {
                std::mem::swap(&mut *lock, &mut tmp);
            }
        }

        // make sure the above lock is released before this is dropped
        drop(tmp);
    }

    fn get_data_chan_id(&self) -> Result<usize> {
        data_chan_strong_core!(self.0, core, { Ok(core.data_chan_id) })
    }

    /// Get the label of this DataChannel.
    #[inline]
    pub fn label(&self) -> Result<GoBuf> {
        unsafe { Ok(GoBuf(API.data_chan_label(self.get_data_chan_id()?)?)) }
    }

    /// Get the ready state of this DataChannel.
    #[inline]
    pub fn ready_state(&self) -> Result<usize> {
        unsafe { API.data_chan_ready_state(self.get_data_chan_id()?) }
    }

    /// Set the buffered amount low threshold.
    /// Returns the current BufferedAmount.
    #[inline]
    pub fn set_buffered_amount_low_threshold(
        &self,
        threshold: usize,
    ) -> Result<usize> {
        unsafe {
            API.data_chan_set_buffered_amount_low_threshold(
                self.get_data_chan_id()?,
                threshold,
            )
        }
    }

    /// Returns the current BufferedAmount.
    #[inline]
    pub fn buffered_amount(&self) -> Result<usize> {
        unsafe { API.data_chan_buffered_amount(self.get_data_chan_id()?) }
    }

    /// Send data to the remote peer on this DataChannel.
    /// Returns the current BufferedAmount.
    pub async fn send<'a, B>(&self, data: B) -> Result<usize>
    where
        B: Into<GoBufRef<'a>>,
    {
        // TODO - use OnBufferedAmountLow signal to implement backpressure
        let data_chan_id = self.get_data_chan_id()?;

        r2id!(data);
        tokio::task::spawn_blocking(move || unsafe {
            API.data_chan_send(data_chan_id, data)
        })
        .await?
    }
}
