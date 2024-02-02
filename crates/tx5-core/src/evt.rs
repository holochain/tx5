use crate::{Error, Result};
use std::sync::Arc;

/// Permit for sending on the channel.
pub struct EventPermit(Option<tokio::sync::OwnedSemaphorePermit>);

impl std::fmt::Debug for EventPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventPermit").finish()
    }
}

/// Sender side of an explicitly bounded channel that lets us send
/// bounded (backpressured) events, but unbounded error messages.
pub struct EventSend<E: From<Error>> {
    limit: Arc<tokio::sync::Semaphore>,
    send: tokio::sync::mpsc::UnboundedSender<(E, EventPermit)>,
}

impl<E: From<Error>> Clone for EventSend<E> {
    fn clone(&self) -> Self {
        Self {
            limit: self.limit.clone(),
            send: self.send.clone(),
        }
    }
}

impl<E: From<Error>> EventSend<E> {
    /// Construct a new event channel with given bound.
    pub fn new(limit: u32) -> (Self, EventRecv<E>) {
        let limit = Arc::new(tokio::sync::Semaphore::new(limit as usize));
        let (send, recv) = tokio::sync::mpsc::unbounded_channel();
        (EventSend { limit, send }, EventRecv(recv))
    }

    /// Try to get a send permit.
    pub fn try_permit(&self) -> Option<EventPermit> {
        match self.limit.clone().try_acquire_owned() {
            Ok(p) => Some(EventPermit(Some(p))),
            _ => None,
        }
    }

    /// Send an event.
    pub async fn send(&self, evt: E) -> Result<()> {
        let permit = self
            .limit
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::id("Closed"))?;
        self.send
            .send((evt, EventPermit(Some(permit))))
            .map_err(|_| Error::id("Closed"))
    }

    /// Send an event with a previously acquired permit.
    pub fn send_permit(&self, evt: E, permit: EventPermit) -> Result<()> {
        self.send
            .send((evt, permit))
            .map_err(|_| Error::id("Closed"))
    }

    /// Send an error.
    pub fn send_err(&self, err: impl Into<Error>) {
        let _ = self.send.send((err.into().into(), EventPermit(None)));
    }
}

/// Receiver side of an explicitly bounded channel that lets us send
/// bounded (backpressured) events, but unbounded error messages.
pub struct EventRecv<E: From<Error>>(
    tokio::sync::mpsc::UnboundedReceiver<(E, EventPermit)>,
);

impl<E: From<Error>> std::fmt::Debug for EventRecv<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventRecv").finish()
    }
}

impl<E: From<Error>> EventRecv<E> {
    /// Receive incoming PeerConnection events.
    pub async fn recv(&mut self) -> Option<E> {
        self.0.recv().await.map(|r| r.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn event_limit() {
        let (s, _r) = <EventSend<()>>::new(1);

        s.send(()).await.unwrap();

        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(10),
            s.send(()),
        )
        .await
        .is_err());
    }
}
