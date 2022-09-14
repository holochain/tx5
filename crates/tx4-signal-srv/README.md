# tx4-signal-srv

Holochain webrtc signal server.

[![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
[![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
[![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

License: MIT OR Apache-2.0

### The `tx4-signal-srv` executable
`tx4-signal-srv --help`
```text
tx4-signal-srv 0.0.1
Holochain Webrtc Signal Server

USAGE:
    tx4-signal-srv [OPTIONS]

OPTIONS:
    -c, --config <CONFIG>            Configuration file to use for running the tx4-signal-srv.
                                     Defaults to `$user_config_dir_path$/tx4-signal-srv.json`
    -h, --help                       Print help information
    -i, --init                       Initialize a new tx4-signal-srv.json configuration file (as
                                     specified by --config). Will abort if it already exists
        --run-with-init-if-needed    Run the signal server, generating a config file if one does not
                                     already exist. Exclusive with "init" option
    -V, --version                    Print version information

```
