# build context
FROM alpine:3.16 AS build

# build the executable
RUN apk add --no-cache build-base cargo git \
  && git clone https://github.com/holochain/tx4.git \
  && cargo build --release --manifest-path tx4/crates/tx4-signal-core/Cargo.toml

# run context
FROM alpine:3.16

# dependency
RUN apk add --no-cache libgcc

# copy executable from build context
COPY --from=build \
  ./tx4/target/release/tx4-signal-srv /usr/bin/tx4-signal-srv

# set executable as entrypoint
ENTRYPOINT ["tx4-signal-srv"]
