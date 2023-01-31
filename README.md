# tx5

Tx5 Holochain WebRTC P2P Communication Ecosystem

- :warning: This code is new and should not yet be considered secure for production use!

[![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
[![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
[![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

## Crates Managed Within This Monorepo

### Tx5

- [tx5](crates/tx5) - The main holochain tx5 webrtc networking crate integrating the other code in this monorepo.

### Tx5 Support Crates

- [tx5-core](crates/tx5-core) - Core types used in other tx5 crates.
- [tx5-go-pion-sys](crates/tx5-go-pion-sys) - Low level rust bindings to the go pion webrtc library.
- [tx5-go-pion](crates/tx5-go-pion) - Higher level rust bindings to the go pion webrtc library.
- [tx5-signal](crates/tx5-signal) - Holochain webrtc signal client.
- [tx5-signal-srv](crates/tx5-signal-srv) - Holochain webrtc signal server.
- [tx5-demo](crates/tx5-demo) - Demo showing off tx5 p2p connectivity.
