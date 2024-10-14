use super::*;

use std::sync::{Arc, Mutex, Weak};

pub fn default_config() -> serde_json::Value {
    serde_json::json!({})
}

pub async fn connect(
    _config: &Arc<Config>,
    _url: &str,
    _listener: bool,
) -> Result<(DynBackEp, DynBackEpRecvCon)> {
    let (ep, ep_recv) = STAT.listen();
    let ep: DynBackEp = ep;
    let ep_recv: DynBackEpRecvCon = Box::new(ep_recv);

    Ok((ep, ep_recv))
}

struct ConRecvData(tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>);

impl BackConRecvData for ConRecvData {
    fn recv(&mut self) -> BoxFuture<'_, Option<Vec<u8>>> {
        Box::pin(async { self.0.recv().await })
    }
}

struct Con {
    pub_key: PubKey,
    send: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl Con {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(pub_key: PubKey) -> (DynBackCon, DynBackConRecvData) {
        let (send, recv) = tokio::sync::mpsc::unbounded_channel();

        let con: DynBackCon = Arc::new(Self { pub_key, send });

        let con_recv: DynBackConRecvData = Box::new(ConRecvData(recv));

        (con, con_recv)
    }
}

impl BackCon for Con {
    fn send(&self, data: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        let r = self.send.send(data).map_err(|err| {
            std::io::Error::other(format!("send failure ({err:?})"))
        });
        Box::pin(async { r })
    }

    fn pub_key(&self) -> &PubKey {
        &self.pub_key
    }

    fn is_using_webrtc(&self) -> bool {
        true
    }

    fn get_stats(&self) -> tx5_connection::ConnStats {
        tx5_connection::ConnStats {
            send_msg_count: 0,
            send_byte_count: 0,
            recv_msg_count: 0,
            recv_byte_count: 0,
        }
    }
}

struct WaitCon {
    pub_key: PubKey,
    con: Option<DynBackCon>,
    con_recv: Option<DynBackConRecvData>,
}

impl BackWaitCon for WaitCon {
    fn wait(
        &mut self,
        _recv_limit: Arc<tokio::sync::Semaphore>,
    ) -> BoxFuture<'static, Result<(DynBackCon, DynBackConRecvData)>> {
        let con = self.con.take().unwrap();
        let con_recv = self.con_recv.take().unwrap();
        Box::pin(async { Ok((con, con_recv)) })
    }

    fn pub_key(&self) -> &PubKey {
        &self.pub_key
    }
}

struct EpRecvCon(tokio::sync::mpsc::UnboundedReceiver<DynBackWaitCon>);

impl BackEpRecvCon for EpRecvCon {
    fn recv(&mut self) -> BoxFuture<'_, Option<DynBackWaitCon>> {
        Box::pin(async { self.0.recv().await })
    }
}

fn gen_pub_key(uniq: u64, loc: usize) -> PubKey {
    // in base64 a string of 0xb7 renders as `t7e3t7e3t...`
    // which sort-of looks like "test" if you squint : )
    let mut pub_key = [0xb7; 32];
    pub_key[16..24].copy_from_slice(&uniq.to_be_bytes());
    pub_key[24..32].copy_from_slice(&(loc as u64).to_be_bytes());

    PubKey(Arc::new(pub_key))
}

fn parse_pub_key(pub_key: &PubKey) -> (u64, usize) {
    let mut uniq = [0; 8];
    uniq.copy_from_slice(&(*pub_key)[16..24]);
    let uniq = u64::from_be_bytes(uniq);

    let mut loc = [0; 8];
    loc.copy_from_slice(&(*pub_key)[24..32]);
    let loc = u64::from_be_bytes(loc) as usize;

    (uniq, loc)
}

struct Ep {
    uniq: u64,
    loc: usize,
    pub_key: PubKey,
    send: tokio::sync::mpsc::UnboundedSender<DynBackWaitCon>,
}

impl Drop for Ep {
    fn drop(&mut self) {
        STAT.remove(self.uniq, self.loc);
    }
}

impl Ep {
    pub fn new(uniq: u64, loc: usize) -> (Arc<Self>, EpRecvCon) {
        let pub_key = gen_pub_key(uniq, loc);

        let (send, recv) = tokio::sync::mpsc::unbounded_channel();

        (
            Arc::new(Self {
                uniq,
                loc,
                pub_key,
                send,
            }),
            EpRecvCon(recv),
        )
    }
}

impl BackEp for Ep {
    fn connect(
        &self,
        pub_key: PubKey,
    ) -> BoxFuture<'_, Result<DynBackWaitCon>> {
        Box::pin(async {
            if self.pub_key == pub_key {
                return Err(std::io::Error::other("cannot connect to self"));
            }
            STAT.connect(self.pub_key.clone(), pub_key)
        })
    }

    fn pub_key(&self) -> &PubKey {
        &self.pub_key
    }
}

struct Stat {
    store: Mutex<slab::Slab<Weak<Ep>>>,
}

impl Stat {
    pub const fn new() -> Self {
        Self {
            store: Mutex::new(slab::Slab::new()),
        }
    }

    pub fn remove(&self, uniq: u64, loc: usize) {
        let mut lock = self.store.lock().unwrap();
        if let Some(ep) = lock.get(loc) {
            if let Some(ep) = ep.upgrade() {
                if ep.uniq == uniq {
                    lock.remove(loc);
                }
            } else {
                lock.remove(loc);
            }
        }
    }

    pub fn listen(&self) -> (Arc<Ep>, EpRecvCon) {
        static UNIQ: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        let uniq = UNIQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let mut lock = self.store.lock().unwrap();
        let entry = lock.vacant_entry();
        let loc = entry.key();
        let (ep, ep_recv) = Ep::new(uniq, loc);
        entry.insert(Arc::downgrade(&ep));

        (ep, ep_recv)
    }

    pub fn connect(
        &self,
        src_pub_key: PubKey,
        dst_pub_key: PubKey,
    ) -> Result<DynBackWaitCon> {
        let (dst_uniq, dst_loc) = parse_pub_key(&dst_pub_key);

        let ep = self.store.lock().unwrap().get(dst_loc).cloned();

        let ep = match ep {
            None => {
                return Err(std::io::Error::other(
                    "failed to connect (no peer)",
                ))
            }
            Some(ep) => ep,
        };

        let ep = match ep.upgrade() {
            None => {
                return Err(std::io::Error::other(
                    "failed to connect (peer closed)",
                ))
            }
            Some(ep) => ep,
        };

        if ep.uniq != dst_uniq {
            return Err(std::io::Error::other(
                "failed to connect (no peer/uniq)",
            ));
        }

        let (dst_con, src_con_recv) = Con::new(dst_pub_key.clone());
        let (src_con, dst_con_recv) = Con::new(src_pub_key.clone());

        let dst_wait: DynBackWaitCon = Box::new(WaitCon {
            pub_key: dst_pub_key,
            con: Some(dst_con),
            con_recv: Some(dst_con_recv),
        });

        let src_wait: DynBackWaitCon = Box::new(WaitCon {
            pub_key: src_pub_key,
            con: Some(src_con),
            con_recv: Some(src_con_recv),
        });

        if ep.send.send(src_wait).is_err() {
            return Err(std::io::Error::other(
                "failed to connect (chan closed)",
            ));
        }

        Ok(dst_wait)
    }
}

static STAT: Stat = Stat::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_dropped_from_slab() {
        let (ep, _recv) = STAT.listen();
        let (uniq, loc) = parse_pub_key(&ep.pub_key);
        drop(ep);
        if let Some(ep) = STAT.store.lock().unwrap().get(loc) {
            if let Some(ep) = ep.upgrade() {
                if ep.uniq == uniq {
                    panic!("failed to delete from slab");
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mem_backend_e2e() {
        let config = Arc::new(Config {
            backend_module: BackendModule::Mem,
            ..Default::default()
        });

        let (e1, _er1) = connect(&config, "", true).await.unwrap();
        let (e2, mut er2) = connect(&config, "", true).await.unwrap();

        assert_ne!(e1.pub_key(), e2.pub_key());

        let mut w1 = e1.connect(e2.pub_key().clone()).await.unwrap();

        assert_eq!(w1.pub_key(), e2.pub_key());

        let (c1, _cr1) = w1
            .wait(Arc::new(tokio::sync::Semaphore::new(1)))
            .await
            .unwrap();

        assert_eq!(c1.pub_key(), e2.pub_key());

        let mut w2 = er2.recv().await.unwrap();

        assert_eq!(w2.pub_key(), e1.pub_key());

        let (c2, mut cr2) = w2
            .wait(Arc::new(tokio::sync::Semaphore::new(1)))
            .await
            .unwrap();

        assert_eq!(c2.pub_key(), e1.pub_key());

        c1.send(vec![1]).await.unwrap();

        let r = cr2.recv().await.unwrap();

        assert_eq!(vec![1], r);

        drop(c1);

        assert_eq!(None, cr2.recv().await);
    }
}
