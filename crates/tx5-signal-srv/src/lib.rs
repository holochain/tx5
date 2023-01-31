#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]

//! Holochain webrtc signal server.
//!
//! - :warning: This code is new and should not yet be considered secure for production use!
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

#![doc = include_str!("docs/srv_help.md")]

/// Re-exported dependencies.
pub mod deps {
    pub use tx5_core::deps::*;
}

use once_cell::sync::Lazy;

static METRICS_REQ_COUNT: Lazy<prometheus::IntCounter> = Lazy::new(|| {
    prometheus::register_int_counter!(
        "metrics_req_cnt",
        "metrics request count"
    )
    .unwrap()
});

static METRICS_REQ_TIME_S: Lazy<prometheus::Histogram> = Lazy::new(|| {
    prometheus::register_histogram!(
        "metrics_req_time_s",
        "metrics request time in seconds"
    )
    .unwrap()
});

static CLIENT_ACTIVE_WS_COUNT: Lazy<prometheus::IntGauge> = Lazy::new(|| {
    prometheus::register_int_gauge!(
        "client_active_ws_cnt",
        "currently active websocket connection count"
    )
    .unwrap()
});

static CLIENT_WS_COUNT: Lazy<prometheus::IntCounter> = Lazy::new(|| {
    prometheus::register_int_counter!(
        "client_ws_cnt",
        "incoming websocket connection count"
    )
    .unwrap()
});

static CLIENT_AUTH_WS_COUNT: Lazy<prometheus::IntCounter> = Lazy::new(|| {
    prometheus::register_int_counter!(
        "client_auth_ws_cnt",
        "incoming websocket connection count that complete authentication"
    )
    .unwrap()
});

static CLIENT_WS_REQ_TIME_S: Lazy<prometheus::Histogram> = Lazy::new(|| {
    prometheus::register_histogram!(
        "client_ws_req_time_s",
        "client websocket request time in seconds"
    )
    .unwrap()
});

static REQ_FWD_CNT: Lazy<prometheus::IntCounter> = Lazy::new(|| {
    prometheus::register_int_counter!(
        "req_fwd_cnt",
        "total count of forward requests processed"
    )
    .unwrap()
});

static REQ_DEMO_CNT: Lazy<prometheus::IntCounter> = Lazy::new(|| {
    prometheus::register_int_counter!(
        "req_demo_cnt",
        "total count of demo broadcast requests processed"
    )
    .unwrap()
});

pub use tx5_core::{Error, ErrorExt, Id, Result};

use clap::Parser;

mod config;
pub use config::*;

mod server;
pub use server::*;
