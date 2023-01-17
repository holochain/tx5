# Build Docker Image

```shell
docker build -f ./Dockerfile ../../.. -t tx4-signal-srv
```

# Run, Generating Config File as Needed

```shell
docker run \
  -t --rm \
  -u $(id -u ${USER}):$(id -g ${USER}) \
  -i --mount type=bind,source="$(pwd)",target=/app/storage \
  --expose 8443 -p 443:8443 \
  --env RUST_LOG=trace \
  tx4-signal-srv \
    --config /app/storage/tx4-signal-srv-config.json \
    --run-with-init-if-needed
```
