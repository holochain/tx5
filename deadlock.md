I'll scan the codebase for lock usage and patterns that commonly lead to deadlocks (e.g., awaiting while holding a lock, blocking within async, nested locks). Then I'll dive into the most suspicious sites.
Ran tool
Ran tool
Ran tool
Ran tool
Ran tool

I'll scan both crates for lock usage and patterns like awaiting while holding a lock, nested locks, and blocking in async contexts. Running targeted searches in parallel now.
Ran tool
Ran tool
Ran tool
Ran tool
Ran tool

I'm going to open the relevant files to inspect where locks are held across awaits or nested with other locks, focusing on `crates/tx5-connection/src/framed.rs`, `crates/tx5-connection/src/lib.rs`, and `crates/tx5/src/sig.rs` and `crates/tx5/src/peer/maybe_ready.rs`.
Read file: /home/thetasinner/source/holo/tx5/crates/tx5-connection/src/framed.rs

I'll open `hub.rs` and `lib.rs` to inspect lock usage near async awaits and in tasks, to spot any held locks across await points.
Read file: /home/thetasinner/source/holo/tx5/crates/tx5-connection/src/hub.rs
Read file: /home/thetasinner/source/holo/tx5/crates/tx5-connection/src/lib.rs
Read file: /home/thetasinner/source/holo/tx5/crates/tx5/src/sig.rs
Read file: /home/thetasinner/source/holo/tx5/crates/tx5/src/peer/maybe_ready.rs

Ran tool

I'll scan for other lock usage patterns across the two crates, focusing on `lock().unwrap()` used near `.await`, and other blocking in async contexts to identify more potential deadlocks.
Ran tool
Ran tool
Ran tool

I'll open `crates/tx5/src/ep.rs` and `crates/tx5/src/peer.rs` around the suspicious areas to see if any `Mutex` guards are kept across `await` or if event loops call back holding locks, which can deadlock.
Read file: /home/thetasinner/source/holo/tx5/crates/tx5/src/ep.rs
Read file: /home/thetasinner/source/holo/tx5/crates/tx5/src/peer.rs

Read file: /home/thetasinner/source/holo/tx5/crates/tx5/src/sig.rs

I'll open `crates/tx5-connection/src/conn.rs` to inspect `Conn::priv_new` and locking behavior to spot potential deadlocks with `webrtc_config` mutex and other channels.
Read file: /home/thetasinner/source/holo/tx5/crates/tx5-connection/src/conn.rs

I scanned both crates for mutex usage, awaits, and channel backpressure. Here are the most credible deadlock/stall hazards and why.

### tx5-connection

- FramedConn holds a tokio Mutex across multiple awaits
  - Why risky: awaiting while holding a `tokio::sync::Mutex` can deadlock if any awaited path tries to re-enter `send` (re-entrancy) or otherwise needs that mutex; it also head-of-line blocks all other sends (including small frames) until a big framed send gets a remote permit, which can look like a deadlock.
```254:296:crates/tx5-connection/src/framed.rs
async fn send_inner(&self, msg: Vec<u8>) -> Result<()> {
    let conn = self.conn.lock().await;

    match crate::proto::proto_encode(&msg)? {
        crate::proto::ProtoEncodeResult::OneMessage(msg) => {
            conn.send(msg).await?;
        }
        crate::proto::ProtoEncodeResult::NeedPermit { permit_req, msg_payload } => {
            let (s_reg, r_reg) = tokio::sync::oneshot::channel();
            let (s_perm, r_perm) = tokio::sync::oneshot::channel();

            self.cmd_send
                .send(Cmd::AwaitPermit { await_registered: s_reg, got_permit: s_perm })
                .await
                .map_err(|_| Error::other("closed"))?;

            r_reg.await.map_err(|_| Error::other("closed"))?;
            conn.send(permit_req).await?;
            r_perm.await.map_err(|_| Error::other("closed"))?;
            for msg in msg_payload {
                conn.send(msg).await?;
            }
        }
    }

    Ok(())
}
```

- Framed command loop can block on delivering to the receiver channel
  - Why risky: if the consumer of `FramedConnRecv` isn’t reading, `msg_send.send(msg).await` will stall the command loop. While stalled, it cannot process `AwaitPermit` or `RemotePermit` commands, so `send_inner` waits forever on those oneshots, appearing as a deadlock.
```100:111:crates/tx5-connection/src/framed.rs
tracing::trace!(...);
if msg_send.send(msg).await.is_err() {
    tracing::info!("FramedConnRecv closed, stopping cmd task");
    break;
}
```

- Long-lived blocking around a std::sync::Mutex under async send path
  - Why risky: `CloseSend::send` wraps an async send in a 60s timeout and uses a `std::sync::Mutex` to guard the sender. While it releases the lock before `.await`, any refactor or additional locking around the `send` path can introduce real blocking. In overload, this will look like deadlock due to the long timeout.
```108:144:crates/tx5-connection/src/lib.rs
pub async fn send(&self, t: T) -> Result<()> {
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        async {
            let sender = self.sender.lock().unwrap().clone();
            if let Some(mut sender) = sender {
                use futures::sink::SinkExt;
                if sender.send(t).await.is_ok() {
                    Result::Ok(())
                } else {
                    Err(ErrorKind::BrokenPipe.into())
                }
            } else {
                Err(ErrorKind::BrokenPipe.into())
            }
        },
    ).await;
    ...
}
```

### tx5

- Locking `EpInner` during drops (lock-in-drop hazard)
  - Why risky: both peer and signal drop guards lock `EpInner` in their `Drop` implementations. If any code path were to hold `EpInner`’s `std::sync::Mutex` across an `.await` (now or via refactor), drop could block on the same mutex while the waiter is blocked on progress that requires the dropped task, causing a classic deadlock. Current code avoids holding the lock across awaits, but this is a sharp edge.
```175:181:crates/tx5/src/peer.rs
impl Drop for DropPeer {
    fn drop(&mut self) {
        ...
        if let Some(ep_inner) = self.ep.upgrade() {
            ep_inner.lock().unwrap().drop_peer_url(&self.peer_url);
        }
        ...
    }
}
```

```161:165:crates/tx5/src/sig.rs
impl Drop for DropSig {
    fn drop(&mut self) {
        ...
        if let Some(inner) = self.inner.upgrade() {
            if let Some(sig) = self.sig.upgrade() {
                inner.lock().unwrap().drop_sig(sig);
            }
        }
    }
}
```

- Event sends in the signal task can block the whole task
  - Why risky: the signal loop awaits `evt_send.send(...).await`. If the application-side receiver isn’t draining events, the signal task stalls. If other parts of the system are waiting on the signal task (e.g., reconnection, permit negotiation pathways elsewhere), this manifests as a deadlock-like stall.
```210:217:crates/tx5/src/sig.rs
if listener {
    let _ = evt_send
        .send(EndpointEvent::ListeningAddressOpen { local_url: local_url.clone() })
        .await;
}
```

What I’d change first
- In `FramedConn::send_inner`, lock only to clone the `Arc<Conn>` and drop the guard before any awaits.
- In the framed cmd loop, consider making `msg_send` unbounded or decouple delivery from command processing so command handling can’t be back-pressured by the consumer.
- Audit any future edits to avoid holding `EpInner`’s `std::sync::Mutex` across awaits, given the lock-in-drop usage.
- Where possible, prefer `tokio::sync::Mutex` for state touched in async functions, or redesign to avoid needing a mutex at all.

I can draft safer edits for `FramedConn::send_inner` and a non-blocking framed cmd loop if you want.

- I identified the locking hot spots and the specific await/lock and channel backpressure patterns that could deadlock or stall.
- Next I can propose concrete edits for `framed.rs` to remove the awaited critical section and to decouple message forwarding from command processing.