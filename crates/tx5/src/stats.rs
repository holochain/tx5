//! Return types for get_stats() call.

/// Backend type.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum StatsBackend {
    /// The rust ffi bindings to the golang pion webrtc library.
    BackendGoPion,

    /// The rust webrtc library.
    BackendWebrtcRs,
}

/// Data for an individual connection.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct StatsConnection {
    /// The public key of the remote peer.
    pub pub_key: [u8; 32],

    /// The message count sent on this connection.
    pub send_message_count: u64,

    /// The bytes sent on this connection.
    pub send_bytes: u64,

    /// The message count received on this connection.
    pub recv_message_count: u64,

    /// The bytes received on this connection
    pub recv_bytes: u64,

    /// UNIX epoch timestamp in seconds when this connection was opened.
    pub opened_at_s: u64,

    /// True if this connection has successfully upgraded to webrtc.
    pub is_webrtc: bool,
}

impl std::fmt::Debug for StatsConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use base64::Engine;
        let k = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(self.pub_key);
        f.debug_struct("StatsConnection")
            .field("pub_key", &k)
            .field("send_message_count", &self.send_message_count)
            .field("send_bytes", &self.send_bytes)
            .field("recv_message_count", &self.recv_message_count)
            .field("recv_bytes", &self.recv_bytes)
            .field("opened_at_s", &self.opened_at_s)
            .field("is_webrtc", &self.is_webrtc)
            .finish()
    }
}

/// Stats type.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Stats {
    /// Backend type.
    pub backend: StatsBackend,

    /// List of PeerUrls this endpoint can currently be reached at.
    pub peer_url_list: Vec<crate::PeerUrl>,

    /// List of current connections.
    pub connection_list: Vec<StatsConnection>,
}
