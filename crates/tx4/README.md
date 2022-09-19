# tx4

Tx4 - The main holochain tx4 webrtc networking crate.

[![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
[![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
[![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

## WebRTC Backend Features

Tx4 can be backed currently by 1 of 2 backend webrtc libraries.

- <b><i>`*`DEFAULT`*`</i></b> `backend-go-pion` - The pion webrtc library
  writen in go (golang).
  - [https://github.com/pion/webrtc](https://github.com/pion/webrtc)
- `backend-webrtc-rs` - The rust webrtc library.
  - [https://github.com/webrtc-rs/webrtc](https://github.com/webrtc-rs/webrtc)

The go pion library is currently the default as it is more mature
and well tested, but comes with some overhead of calling into a different
memory/runtime. When the rust library is stable enough for holochain's
needs, we will switch the default. To switch now, or if you want to
make sure the backend doesn't change out from under you, set
no-default-features and explicitly enable the backend of your choice.

License: MIT OR Apache-2.0
