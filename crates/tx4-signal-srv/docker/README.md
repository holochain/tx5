# Build Docker Image

```shell
docker build . -t tx4-signal-srv
```

# Generate Config File

```shell
docker run \
  -t --rm \
  -u $(id -u ${USER}):$(id -g ${USER}) \
  -i --mount type=bind,source="$(pwd)",target=/host \
  --expose 8443 -p 443:8443 \
  --env RUST_LOG=trace \
  tx4-signal-srv --config /host/config.json --init
```

# Edit Config File

Setup the `"bildingList"` array to match the server:

```json
  "bindingList": [
    {
      "localInterface": "<my ipv4 iface addr>:8443",
      "wanHost": "<my global ipv4 address>",
      "wanPort": 443,
      "enabled": true,
      "notes": []
    },
    {
      "localInterface": "[<my ipv6 iface addr>]:8443",
      "wanHost": "<my global ipv6 address>",
      "wanPort": 443,
      "enabled": true,
      "notes": []
    },
  ],
```

# Run the Server

```shell
docker run \
  -t --rm \
  -u $(id -u ${USER}):$(id -g ${USER}) \
  -i --mount type=bind,source="$(pwd)",target=/host \
  --expose 8443 -p 443:8443 \
  --env RUST_LOG=trace \
  tx4-signal-srv --config /host/config.json
```
