use crate::*;

pub struct ImpChanSeed {
    seed: tx4_go_pion::DataChannelSeed,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl ImpChanSeed {
    pub fn new(seed: tx4_go_pion::DataChannelSeed) -> Self {
        Self {
            seed,
            _not_sync: std::marker::PhantomData,
        }
    }

    pub async fn handle<Cb>(self, cb: Cb) -> Result<ImpChan>
    where
        Cb: Fn(DataChannelEvent) + 'static + Send + Sync,
    {
        let ImpChanSeed { seed, .. } = self;
        let (open_snd, mut open_rcv) = tokio::sync::mpsc::unbounded_channel();
        let mut chan = seed.handle(move |evt| match evt {
            tx4_go_pion::DataChannelEvent::Open => {
                let _ = open_snd.send(());
            }
            tx4_go_pion::DataChannelEvent::Close => {
                cb(DataChannelEvent::Close);
            }
            tx4_go_pion::DataChannelEvent::Message(buf) => {
                cb(DataChannelEvent::Message(Buf {
                    imp: crate::buf::imp::Imp {
                        buf,
                        _not_sync: std::marker::PhantomData,
                    },
                    _not_sync: std::marker::PhantomData,
                }));
            }
        });
        if chan.ready_state()? < 2 {
            // if we're not ready yet, wait for Open event
            let _ = open_rcv.recv().await;
        }
        Ok(ImpChan {
            chan,
            _not_sync: std::marker::PhantomData,
        })
    }
}

pub struct ImpChan {
    chan: tx4_go_pion::DataChannel,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl ImpChan {
    pub fn label(&mut self) -> Result<Vec<u8>> {
        self.chan.label()?.to_vec()
    }

    pub fn is_closed(&mut self) -> Result<bool> {
        Ok(self.chan.ready_state()? > 2)
    }

    pub async fn send<'a, B>(&mut self, data: B) -> Result<()>
    where
        B: Into<&'a mut Buf>,
    {
        let data = data.into();
        self.chan.send(&mut data.imp.buf).await
    }
}
