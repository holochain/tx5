# build context
FROM alpine:3.17 AS build

# include the source tree for build
ADD Cargo.toml /Cargo.toml
ADD crates /crates

# build the executable
RUN apk add --no-cache build-base cargo \
  && cargo build --release --manifest-path crates/tx5-signal-srv/Cargo.toml

# run context
FROM alpine:3.17

# dependency
RUN apk add --no-cache libgcc

# copy executable from build context
COPY --from=build \
  ./target/release/tx5-signal-srv /usr/bin/tx5-signal-srv

# set executable as entrypoint
ENTRYPOINT ["tx5-signal-srv"]
