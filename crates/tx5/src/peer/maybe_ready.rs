use crate::backend::DynBackCon;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct MaybeReady(Arc<Mutex<MaybeReadyState>>);

enum MaybeReadyState {
    Ready(DynBackCon),
    Failed,
    Wait(Arc<tokio::sync::Semaphore>),
}

impl MaybeReady {
    pub(super) fn new() -> Self {
        MaybeReady(Arc::new(Mutex::new(MaybeReadyState::Wait(Arc::new(
            tokio::sync::Semaphore::new(0),
        )))))
    }

    pub(super) fn query_ready<T>(
        &self,
        query: fn(&DynBackCon) -> T,
    ) -> Option<T> {
        let lock = self.0.lock().expect("poisoned");
        match &*lock {
            MaybeReadyState::Ready(c) => Some(query(c)),
            _ => None,
        }
    }

    pub(super) fn set_ready(&self, conn: DynBackCon) {
        let mut lock = self.0.lock().expect("poisoned");
        match &*lock {
            MaybeReadyState::Wait(w) => {
                // Close the wait semaphore to unblock waiters.
                w.close();
            }
            MaybeReadyState::Failed => {
                tracing::error!(
                    "Cannot set state to ready because state is already failed"
                );
                return;
            }
            MaybeReadyState::Ready(_) => {
                tracing::error!(
                    "Cannot set state to ready because state is already ready"
                );
                return;
            }
        }

        *lock = MaybeReadyState::Ready(conn);
    }

    pub(super) fn set_failed(&self) {
        let mut lock = self.0.lock().expect("poisoned");
        match &*lock {
            MaybeReadyState::Wait(w) => {
                // Close the wait semaphore to unblock waiters.
                w.close();
            }
            MaybeReadyState::Failed => {
                // Don't warn about duplicate failed calls, it's a no-op and not doing any harm.
                return;
            }
            MaybeReadyState::Ready(_) => {
                // Have to quietly permit this too because it's used in a drop implementation which
                // can happen either before or after the connection was marked ready.
                return;
            }
        }

        *lock = MaybeReadyState::Failed;
    }

    pub(super) async fn wait_for_ready(&self) -> Option<DynBackCon> {
        let wait = {
            let lock = self.0.lock().expect("poisoned");
            match &*lock {
                MaybeReadyState::Ready(back) => return Some(back.clone()),
                MaybeReadyState::Failed => return None,
                MaybeReadyState::Wait(wait) => wait.clone(),
            }
        };

        let _ = wait.acquire().await;

        let lock = self.0.lock().expect("poisoned");
        match &*lock {
            MaybeReadyState::Ready(back) => Some(back.clone()),
            MaybeReadyState::Failed => None,
            MaybeReadyState::Wait(_) => {
                // Unexpected state, this struct is supposed to prevent this being possible.
                tracing::warn!("Waited for ready but still in a wait state");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackCon;
    use crate::PubKey;
    use futures::future::BoxFuture;
    use tx5_connection::ConnStats;

    struct NoopCon {
        pub_key: PubKey,
    }

    impl BackCon for NoopCon {
        fn send(&self, _data: Vec<u8>) -> BoxFuture<'_, std::io::Result<()>> {
            Box::pin(async move { Ok(()) })
        }

        fn pub_key(&self) -> &PubKey {
            &self.pub_key
        }

        fn is_using_webrtc(&self) -> bool {
            false
        }

        fn get_stats(&self) -> ConnStats {
            ConnStats::default()
        }
    }

    #[tokio::test]
    async fn ready() {
        let maybe_ready = MaybeReady::new();

        let (tx, mut rx) = tokio::sync::oneshot::channel();
        tokio::spawn({
            let maybe_ready = maybe_ready.clone();
            async move {
                tx.send(maybe_ready.wait_for_ready().await.is_some())
                    .unwrap();
            }
        });

        // Not ready, should fail to receive.
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut rx)
            .await
            .unwrap_err();

        assert!(
            maybe_ready.query_ready(|c| c.pub_key().clone()).is_none(),
            "Not ready, shouldn't be able to query"
        );

        maybe_ready.set_ready(Arc::new(NoopCon {
            pub_key: PubKey(Arc::new([0; 32])),
        }));

        // Now is ready, should receive boolean true.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), rx)
                .await
                .unwrap()
                .unwrap(),
            "Expected a successful ready"
        );

        assert!(
            maybe_ready.query_ready(|c| c.pub_key().clone()).is_some(),
            "Is ready, should be able to query"
        );
    }

    #[tokio::test]
    async fn failed() {
        let maybe_ready = MaybeReady::new();

        let (tx, mut rx) = tokio::sync::oneshot::channel();
        tokio::spawn({
            let maybe_ready = maybe_ready.clone();
            async move {
                tx.send(maybe_ready.wait_for_ready().await.is_none())
                    .unwrap();
            }
        });

        // Not failed, should fail to receive.
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut rx)
            .await
            .unwrap_err();

        maybe_ready.set_failed();

        // Now is failed, should receive boolean true.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), rx)
                .await
                .unwrap()
                .unwrap(),
            "Expected a failed state"
        );

        assert!(
            maybe_ready.query_ready(|c| c.pub_key().clone()).is_none(),
            "Is failed, shouldn't be able to query"
        );
    }
}
