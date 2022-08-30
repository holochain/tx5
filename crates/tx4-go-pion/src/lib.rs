#![deny(missing_docs)]
#![deny(warnings)]

//! Higher level rust bindings to the go pion webrtc library.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

/// Re-exported dependencies.
pub mod deps {
    pub use libc;
    pub use once_cell;
    pub use tempfile;
    pub use tx4_core::deps::*;
    pub use tx4_go_pion_sys;
    pub use tx4_go_pion_sys::deps::*;
}

use deps::*;

pub use tx4_core::{Error, ErrorExt, Id, Result};

mod evt;
pub use evt::*;

mod go_buf;
pub use go_buf::*;

mod peer_con;
pub use peer_con::*;

mod data_chan;
pub use data_chan::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const STUN: &str = r#"{
    "iceServers": [
        {
          "urls": ["stun:openrelay.metered.ca:80"]
        },
        {
          "urls": ["turn:openrelay.metered.ca:80"],
          "username": "openrelayproject",
          "credential": "openrelayproject"
        },
        {
          "urls": ["turn:openrelay.metered.ca:443"],
          "username": "openrelayproject",
          "credential": "openrelayproject"
        },
        {
          "urls": ["turn:openrelay.metered.ca:443?transport=tcp"],
          "username": "openrelayproject",
          "credential": "openrelayproject"
        }
    ]
}"#;

    #[test]
    fn peer_con() {
        let ice1 = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let ice2 = Arc::new(parking_lot::Mutex::new(Vec::new()));

        #[derive(Debug)]
        enum Cmd {
            Shutdown,
            ICE(String),
            Offer(String),
            Answer(String),
        }

        #[derive(Debug)]
        enum Res {
            Chan1(DataChannelSeed),
            Chan2(DataChannelSeed),
        }

        let (cmd_send_1, cmd_recv_1) = std::sync::mpsc::sync_channel(32);
        let cmd_send_1 = Arc::new(cmd_send_1);

        let (cmd_send_2, cmd_recv_2) = std::sync::mpsc::sync_channel(32);
        let cmd_send_2 = Arc::new(cmd_send_2);

        let (res_send, res_recv) = std::sync::mpsc::sync_channel(32);
        let res_send = Arc::new(res_send);

        // -- spawn thread for peer connection 1 -- //

        let hnd1 = {
            let res_send = res_send.clone();
            let cmd_send_2 = cmd_send_2.clone();
            let ice1 = ice1.clone();
            std::thread::spawn(move || {
                let mut peer1 = {
                    let cmd_send_2 = cmd_send_2.clone();
                    PeerConnection::new(STUN, move |evt| match evt {
                        PeerConnectionEvent::ICECandidate(candidate) => {
                            println!("peer1 in-ice: {}", candidate);
                            ice1.lock().push(candidate.clone());
                            cmd_send_2.send(Cmd::ICE(candidate)).unwrap();
                        }
                        PeerConnectionEvent::DataChannel(chan) => {
                            println!("peer1 in-chan: {:?}", chan);
                        }
                    })
                    .unwrap()
                };

                let chan1 = peer1
                    .create_data_channel("{ \"label\": \"data\" }")
                    .unwrap();

                res_send.send(Res::Chan1(chan1)).unwrap();

                let offer = peer1.create_offer(None).unwrap();
                peer1.set_local_description(&offer).unwrap();
                cmd_send_2.send(Cmd::Offer(offer.clone())).unwrap();

                while let Ok(cmd) = cmd_recv_1.recv() {
                    match cmd {
                        Cmd::ICE(ice) => peer1.add_ice_candidate(&ice).unwrap(),
                        Cmd::Answer(answer) => {
                            println!("peer1 recv answer: {}", answer);
                            peer1.set_remote_description(&answer).unwrap();
                        }
                        _ => break,
                    }
                }
            })
        };

        // -- spawn thread for peer connection 2 -- //

        let hnd2 = {
            let res_send = res_send.clone();
            let cmd_send_1 = cmd_send_1.clone();
            let ice2 = ice2.clone();
            std::thread::spawn(move || {
                let mut peer2 = {
                    let cmd_send_1 = cmd_send_1.clone();
                    PeerConnection::new(STUN, move |evt| match evt {
                        PeerConnectionEvent::ICECandidate(candidate) => {
                            println!("peer2 in-ice: {}", candidate);
                            ice2.lock().push(candidate.clone());
                            cmd_send_1.send(Cmd::ICE(candidate)).unwrap();
                        }
                        PeerConnectionEvent::DataChannel(chan) => {
                            println!("peer2 in-chan: {:?}", chan);
                            res_send.send(Res::Chan2(chan)).unwrap();
                        }
                    })
                    .unwrap()
                };

                while let Ok(cmd) = cmd_recv_2.recv() {
                    match cmd {
                        Cmd::ICE(ice) => peer2.add_ice_candidate(&ice).unwrap(),
                        Cmd::Offer(offer) => {
                            println!("peer2 recv offer: {}", offer);
                            peer2.set_remote_description(&offer).unwrap();
                            let answer = peer2.create_answer(None).unwrap();
                            peer2.set_local_description(&answer).unwrap();
                            cmd_send_1.send(Cmd::Answer(answer)).unwrap();
                        }
                        _ => break,
                    }
                }
            })
        };

        // -- retrieve our data channels -- //

        let mut chan1 = None;
        let mut chan2 = None;

        for _ in 0..2 {
            match res_recv.recv().unwrap() {
                Res::Chan1(chan) => chan1 = Some(chan),
                Res::Chan2(chan) => chan2 = Some(chan),
            }
        }

        let (s_open, r_open) = std::sync::mpsc::sync_channel(32);
        let (s_data, r_data) = std::sync::mpsc::sync_channel(32);

        // -- setup event handler for data channel 1 -- //

        let s_open1 = s_open.clone();
        let s_data1 = s_data.clone();
        let mut chan1 = chan1.unwrap().handle(move |evt| {
            println!("chan1: {:?}", evt);
            if let DataChannelEvent::Open = evt {
                s_open1.send(()).unwrap();
            }
            if let DataChannelEvent::Message(mut msg) = evt {
                msg.access(|data| {
                    assert_eq!(b"world", data.unwrap());
                    Ok(())
                })
                .unwrap();
                s_data1.send(()).unwrap();
            }
        });

        // -- setup event handler for data channel 2 -- //

        let mut chan2 = chan2.unwrap().handle(move |evt| {
            println!("chan2: {:?}", evt);
            if let DataChannelEvent::Open = evt {
                s_open.send(()).unwrap();
            }
            if let DataChannelEvent::Message(mut msg) = evt {
                msg.access(|data| {
                    assert_eq!(b"hello", data.unwrap());
                    Ok(())
                })
                .unwrap();
                s_data.send(()).unwrap();
            }
        });

        // -- make sure the channels are ready / open -- //

        let chan1ready = chan1.ready_state().unwrap();
        println!("chan1 ready_state: {}", chan1ready);
        let chan2ready = chan2.ready_state().unwrap();
        println!("chan2 ready_state: {}", chan2ready);

        let mut need_open_cnt = 0;
        if chan1ready < 2 {
            need_open_cnt += 1;
        }
        if chan2ready < 2 {
            need_open_cnt += 1;
        }

        for _ in 0..need_open_cnt {
            r_open.recv().unwrap();
        }

        // -- send data on the data channels -- //

        let mut buf = GoBuf::new().unwrap();
        buf.extend(b"hello").unwrap();
        chan1.send(buf).unwrap();

        let mut buf = GoBuf::new().unwrap();
        buf.extend(b"world").unwrap();
        chan2.send(buf).unwrap();

        // -- await receiving data on the data channels -- //

        for _ in 0..2 {
            r_data.recv().unwrap();
        }

        // -- cleanup -- //

        drop(chan1);
        drop(chan2);
        cmd_send_1.send(Cmd::Shutdown).unwrap();
        cmd_send_2.send(Cmd::Shutdown).unwrap();
        hnd1.join().unwrap();
        hnd2.join().unwrap();
    }
}
