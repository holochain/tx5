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

/*
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    //"urls": [
    //    "stun:stun.l.google.com:19302"
    //]

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
    ],
    "certificates": [
        "-----BEGIN CERTIFICATE-----\nTUlJQklqQ0J5YUFEQWdFQ0FoRUJSVHR5Zk8vNFF5dS9OU3o3UU5yeExEQUtCZ2dx\naGtqT1BRUURBakFSTVE4d0RRWURWUVFERXdaWFpXSlNWRU13SGhjTk1qSXdOekEy\nTVRreE1EVTNXaGNOTWpJd09EQTJNVGt4TURVM1dqQVJNUTh3RFFZRFZRUURFd1pY\nWldKU1ZFTXdXVEFUQmdjcWhrak9QUUlCQmdncWhrak9QUU1CQndOQ0FBU3phSGxZ\nSW1ORHJsN1JFYnJLS0ZyN2lwSjJkNERRZXJOMXV1WjBZeGhadExzOXNBN1Zid1A4\nSGxBNktrZjV4MGcvRFFDVU56UytXb1U0eFFOTkFBL0xvd0l3QURBS0JnZ3Foa2pP\nUFFRREFnTklBREJGQWlFQXM2NEVITDIyVHF0U2hKQlVBaTljUm1YTDFqOVJHbnlT\nQ0loRFBCTnFHcjBDSUJ5T2d3L2x0bitmU01yMnVYczdDRmJaT0Rla0ZIUlNPTzZW\nLzBCSGp6dzg=\n-----END CERTIFICATE-----\n-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg9Cems82+pSDGMsnS\nfjpQbZr67x6mg2ClewdtlA8u7eOhRANCAASzaHlYImNDrl7REbrKKFr7ipJ2d4DQ\nerN1uuZ0YxhZtLs9sA7VbwP8HlA6Kkf5x0g/DQCUNzS+WoU4xQNNAA/L\n-----END PRIVATE KEY-----"
    ]
}"#;

    const CERT: &[u8] = &[
        48, 130, 1, 34, 48, 129, 201, 160, 3, 2, 1, 2, 2, 17, 1, 69, 59, 114,
        124, 239, 248, 67, 43, 191, 53, 44, 251, 64, 218, 241, 44, 48, 10, 6,
        8, 42, 134, 72, 206, 61, 4, 3, 2, 48, 17, 49, 15, 48, 13, 6, 3, 85, 4,
        3, 19, 6, 87, 101, 98, 82, 84, 67, 48, 30, 23, 13, 50, 50, 48, 55, 48,
        54, 49, 57, 49, 48, 53, 55, 90, 23, 13, 50, 50, 48, 56, 48, 54, 49, 57,
        49, 48, 53, 55, 90, 48, 17, 49, 15, 48, 13, 6, 3, 85, 4, 3, 19, 6, 87,
        101, 98, 82, 84, 67, 48, 89, 48, 19, 6, 7, 42, 134, 72, 206, 61, 2, 1,
        6, 8, 42, 134, 72, 206, 61, 3, 1, 7, 3, 66, 0, 4, 179, 104, 121, 88,
        34, 99, 67, 174, 94, 209, 17, 186, 202, 40, 90, 251, 138, 146, 118,
        119, 128, 208, 122, 179, 117, 186, 230, 116, 99, 24, 89, 180, 187, 61,
        176, 14, 213, 111, 3, 252, 30, 80, 58, 42, 71, 249, 199, 72, 63, 13, 0,
        148, 55, 52, 190, 90, 133, 56, 197, 3, 77, 0, 15, 203, 163, 2, 48, 0,
        48, 10, 6, 8, 42, 134, 72, 206, 61, 4, 3, 2, 3, 72, 0, 48, 69, 2, 33,
        0, 179, 174, 4, 28, 189, 182, 78, 171, 82, 132, 144, 84, 2, 47, 92, 70,
        101, 203, 214, 63, 81, 26, 124, 146, 8, 136, 67, 60, 19, 106, 26, 189,
        2, 32, 28, 142, 131, 15, 229, 182, 127, 159, 72, 202, 246, 185, 123,
        59, 8, 86, 217, 56, 55, 164, 20, 116, 82, 56, 238, 149, 255, 64, 71,
        143, 60, 60,
    ];

    #[test]
    fn peer_con() {
        let ice1 = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let ice2 = Arc::new(parking_lot::Mutex::new(Vec::new()));

        #[derive(Debug)]
        enum Cmd {
            Shutdown,
            CheckCert,
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
                        Cmd::CheckCert => {
                            let cert = peer1.get_remote_certificate().unwrap();
                            assert_eq!(CERT, &*cert);
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
                        Cmd::CheckCert => {
                            let cert = peer2.get_remote_certificate().unwrap();
                            assert_eq!(CERT, &*cert);
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

        // -- check remote certificates -- //

        cmd_send_1.send(Cmd::CheckCert).unwrap();
        cmd_send_2.send(Cmd::CheckCert).unwrap();

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
*/
