### The `tx5-signal-srv` executable
`tx5-signal-srv --help`
```text
Holochain Webrtc Signal Server

Usage: tx5-signal-srv [OPTIONS]

Options:
  -i, --init                     Initialize a new tx5-signal-srv.json configuration file (as
                                 specified by --config). Will abort if it already exists
      --run-with-init-if-needed  Run the signal server, generating a config file if one does not
                                 already exist. Exclusive with "init" option
  -c, --config <CONFIG>          Configuration file to use for running the tx5-signal-srv. Defaults
                                 to `$user_config_dir_path$/tx5-signal-srv.json`
  -h, --help                     Print help
  -V, --version                  Print version

```
