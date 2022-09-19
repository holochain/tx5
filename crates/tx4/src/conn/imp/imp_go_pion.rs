use crate::*;

pub struct ImpConn {
    conn: tx4_go_pion::PeerConnection,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl ImpConn {
    pub async fn new<'a, B, Cb>(config: B, cb: Cb) -> Result<Self>
    where
        B: Into<BufRef<'a>>,
        Cb: Fn(PeerConnectionEvent) + 'static + Send + Sync,
    {
        let mut config = config.into();
        let config = config.as_mut_ref()?;
        let conn =
            tx4_go_pion::PeerConnection::new(&mut config.imp.buf, move |evt| {
                match evt {
                    tx4_go_pion::PeerConnectionEvent::Error(err) => {
                        panic!("{:?}", err)
                    }
                    tx4_go_pion::PeerConnectionEvent::ICECandidate(buf) => {
                        cb(PeerConnectionEvent::IceCandidate(Buf {
                            imp: buf::imp::Imp {
                                buf,
                                _not_sync: std::marker::PhantomData,
                            },
                            _not_sync: std::marker::PhantomData,
                        }));
                    }
                    tx4_go_pion::PeerConnectionEvent::DataChannel(seed) => {
                        let seed = crate::chan::imp::ImpChanSeed::new(seed);
                        let seed = DataChannelSeed {
                            imp: seed,
                            _not_sync: std::marker::PhantomData,
                        };
                        cb(PeerConnectionEvent::DataChannel(seed));
                    }
                }
            })
            .await?;
        Ok(Self {
            conn,
            _not_sync: std::marker::PhantomData,
        })
    }

    pub async fn create_offer<'a, B>(&mut self, config: B) -> Result<Buf>
    where
        B: Into<BufRef<'a>>,
    {
        let mut config = config.into();
        let config = config.as_mut_ref()?;
        let buf = self.conn.create_offer(&mut config.imp.buf).await?;
        Ok(Buf {
            imp: buf::imp::Imp {
                buf,
                _not_sync: std::marker::PhantomData,
            },
            _not_sync: std::marker::PhantomData,
        })
    }

    pub async fn create_answer<'a, B>(&mut self, config: B) -> Result<Buf>
    where
        B: Into<BufRef<'a>>,
    {
        let mut config = config.into();
        let config = config.as_mut_ref()?;
        let buf = self.conn.create_answer(&mut config.imp.buf).await?;
        Ok(Buf {
            imp: buf::imp::Imp {
                buf,
                _not_sync: std::marker::PhantomData,
            },
            _not_sync: std::marker::PhantomData,
        })
    }

    pub async fn set_local_description<'a, B>(&mut self, desc: B) -> Result<()>
    where
        B: Into<BufRef<'a>>,
    {
        let mut desc = desc.into();
        let desc = desc.as_mut_ref()?;
        self.conn.set_local_description(&mut desc.imp.buf).await
    }

    pub async fn set_remote_description<'a, B>(&mut self, desc: B) -> Result<()>
    where
        B: Into<BufRef<'a>>,
    {
        let mut desc = desc.into();
        let desc = desc.as_mut_ref()?;
        self.conn.set_remote_description(&mut desc.imp.buf).await
    }

    pub async fn add_ice_candidate<'a, B>(&mut self, ice: B) -> Result<()>
    where
        B: Into<BufRef<'a>>,
    {
        let mut ice = ice.into();
        let ice = ice.as_mut_ref()?;
        self.conn.add_ice_candidate(&mut ice.imp.buf).await
    }

    pub async fn create_data_channel<'a, B>(
        &mut self,
        config: B,
    ) -> Result<DataChannelSeed>
    where
        B: Into<BufRef<'a>>,
    {
        let mut config = config.into();
        let config = config.as_mut_ref()?;
        let seed = self.conn.create_data_channel(&mut config.imp.buf).await?;
        let seed = crate::chan::imp::ImpChanSeed::new(seed);
        let seed = DataChannelSeed {
            imp: seed,
            _not_sync: std::marker::PhantomData,
        };
        Ok(seed)
    }
}
