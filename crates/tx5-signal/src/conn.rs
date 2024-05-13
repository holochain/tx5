use crate::*;

/// Tx5 signal connection configuration.
pub type SignalConfig = sbd_e2e_crypto_client::Config;

/// Receive messages from the signal server.
pub struct MsgRecv {
    client: Weak<sbd_e2e_crypto_client::SbdClientCrypto>,
    recv: sbd_e2e_crypto_client::MsgRecv,
}

impl std::ops::Deref for MsgRecv {
    type Target = sbd_e2e_crypto_client::MsgRecv;

    fn deref(&self) -> &Self::Target {
        &self.recv
    }
}

impl std::ops::DerefMut for MsgRecv {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.recv
    }
}

impl MsgRecv {
    /// Receive messages from the signal server.
    pub async fn recv_message(&mut self) -> Option<(PubKey, SignalMessage)> {
        loop {
            let (pub_key, msg) = self.recv.recv().await?;
            match SignalMessage::parse(msg) {
                Err(_) => {
                    if let Some(client) = self.client.upgrade() {
                        client.close_peer(&pub_key).await;
                    }
                    continue;
                }
                Ok(SignalMessage::Unknown) => continue,
                Ok(msg) => return Some((pub_key, msg)),
            }
        }
    }
}

/// A client connection to a tx5 signal server.
pub struct SignalConnection {
    client: Arc<sbd_e2e_crypto_client::SbdClientCrypto>,
}

impl std::ops::Deref for SignalConnection {
    type Target = sbd_e2e_crypto_client::SbdClientCrypto;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl SignalConnection {
    /// Establish a new client connection to a tx5 signal server.
    pub async fn connect(
        url: &str,
        config: Arc<SignalConfig>,
    ) -> Result<(Self, MsgRecv)> {
        let (client, recv) =
            sbd_e2e_crypto_client::SbdClientCrypto::new(url, config).await?;
        let client = Arc::new(client);
        let weak_client = Arc::downgrade(&client);

        Ok((
            Self { client },
            MsgRecv {
                client: weak_client,
                recv,
            },
        ))
    }

    /// Send a handshake request to a peer. Returns the nonce sent.
    pub async fn send_handshake_req(
        &self,
        pub_key: &PubKey,
    ) -> Result<[u8; 32]> {
        let (nonce, msg) = SignalMessage::handshake_req();
        self.client.send(pub_key, &msg).await?;
        Ok(nonce)
    }

    /// Send a handshake response to a peer.
    pub async fn send_handshake_res(
        &self,
        pub_key: &PubKey,
        nonce: [u8; 32],
    ) -> Result<()> {
        let msg = SignalMessage::handshake_res(nonce);
        self.client.send(pub_key, &msg).await?;
        Ok(())
    }

    /// Send a webrtc offer request to a peer.
    pub async fn send_offer_req(&self, pub_key: &PubKey) -> Result<()> {
        let msg = SignalMessage::offer_req();
        self.client.send(pub_key, &msg).await?;
        Ok(())
    }

    /// Send a webrtc offer to a peer.
    pub async fn send_offer(
        &self,
        pub_key: &PubKey,
        offer: Vec<u8>,
    ) -> Result<()> {
        let msg = SignalMessage::offer(offer)?;
        self.client.send(pub_key, &msg).await?;
        Ok(())
    }

    /// Send a webrtc answer to a peer.
    pub async fn send_answer(
        &self,
        pub_key: &PubKey,
        answer: Vec<u8>,
    ) -> Result<()> {
        let msg = SignalMessage::answer(answer)?;
        self.client.send(pub_key, &msg).await?;
        Ok(())
    }

    /// Send a webrtc ice message to a peer.
    pub async fn send_ice(&self, pub_key: &PubKey, ice: Vec<u8>) -> Result<()> {
        let msg = SignalMessage::ice(ice)?;
        self.client.send(pub_key, &msg).await?;
        Ok(())
    }

    /// Send a communication message to a peer.
    pub async fn send_message(
        &self,
        pub_key: &PubKey,
        message: Vec<u8>,
    ) -> Result<()> {
        let msg = SignalMessage::message(message)?;
        self.client.send(pub_key, &msg).await?;
        Ok(())
    }

    /// Keepalive.
    pub async fn send_keepalive(&self, pub_key: &PubKey) -> Result<()> {
        let msg = SignalMessage::keepalive();
        self.client.send(pub_key, &msg).await?;
        Ok(())
    }
}
