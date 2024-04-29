use crate::*;

/// Tx5 signal connection configuration.
pub type SignalConfig = sbd_e2e_crypto_client::Config;

/// A client connection to a tx5 signal server.
pub struct SignalConnection {
    client: Arc<sbd_e2e_crypto_client::SbdClientCrypto>,
}

impl SignalConnection {
    /// Establish a new client connection to a tx5 signal server.
    pub async fn connect(url: &str, config: Arc<SignalConfig>) -> Result<Self> {
        let client =
            sbd_e2e_crypto_client::SbdClientCrypto::new(url, config).await?;
        let client = Arc::new(client);

        Ok(Self { client })
    }

    /// The public key identifier of *this* client node.
    pub fn pub_key(&self) -> &PubKey {
        self.client.pub_key()
    }

    /// Receive a message from a remote peer.
    pub async fn recv(&self) -> Option<(PubKey, SignalMessage)> {
        loop {
            let (pub_key, msg) = self.client.recv().await?;
            match SignalMessage::parse(msg) {
                Err(_) => {
                    self.client.close_peer(&pub_key).await;
                    continue;
                }
                Ok(SignalMessage::Unknown) => continue,
                Ok(msg) => return Some((pub_key, msg)),
            }
        }
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
        offer: serde_json::Value,
    ) -> Result<()> {
        let msg = SignalMessage::offer(offer)?;
        self.client.send(pub_key, &msg).await?;
        Ok(())
    }

    /// Send a webrtc answer to a peer.
    pub async fn send_answer(
        &self,
        pub_key: &PubKey,
        answer: serde_json::Value,
    ) -> Result<()> {
        let msg = SignalMessage::answer(answer)?;
        self.client.send(pub_key, &msg).await?;
        Ok(())
    }

    /// Send a webrtc ice message to a peer.
    pub async fn send_ice(
        &self,
        pub_key: &PubKey,
        ice: serde_json::Value,
    ) -> Result<()> {
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

    /// Close a connection to a specific peer.
    pub async fn close_peer(&self, pub_key: &PubKey) {
        self.client.close_peer(pub_key).await;
    }

    /// Close the entire signal client connection.
    pub async fn close(&self) {
        self.client.close().await;
    }
}
