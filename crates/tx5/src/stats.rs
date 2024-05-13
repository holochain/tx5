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
#[derive(Debug, serde::Serialize, serde::Deserialize)]
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
