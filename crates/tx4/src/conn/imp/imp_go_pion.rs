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
                    tx4_go_pion::PeerConnectionEvent::ICECandidate(buf) => {
                        cb(PeerConnectionEvent::IceCandidate(Buf {
                            imp: buf::imp::Imp {
                                buf,
                                _not_sync: std::marker::PhantomData,
                            },
                            _not_sync: std::marker::PhantomData,
                        }));
                    }
                    _ => panic!("AHHH"),
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
}
