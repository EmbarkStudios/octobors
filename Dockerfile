FROM rust:1.49.0-slim-buster as build

COPY . /src

RUN set -eux; \
    # Install musl-tools so that we can compile with musl libc
    apt-get update && apt-get install -y musl-tools; \
    # Ditto for the rust target
    rustup target add x86_64-unknown-linux-musl; \
    cd /src; \
    cargo build --release --target x86_64-unknown-linux-musl; \
    strip target/x86_64-unknown-linux-musl/release/octobors;

FROM alpine:3.12 as run

COPY --from=build /src/target/x86_64-unknown-linux-musl/release/octobors /usr/local/bin/octobors

ENTRYPOINT ["octobors"]
