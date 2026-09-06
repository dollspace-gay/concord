FROM node:22.23.1-bookworm-slim AS web-builder
WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.96.0-bookworm AS server-builder
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY server/Cargo.toml server/Cargo.toml
COPY server/build.rs server/build.rs
COPY server/src server/src
COPY server/migrations server/migrations
RUN cargo build --workspace --release --locked --bin concord-server --bin concord_operator

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 concord \
    && useradd --uid 10001 --gid concord --no-create-home --shell /usr/sbin/nologin concord \
    && install -d -o concord -g concord -m 0700 /var/lib/concord /var/lib/concord/media /var/lib/concord/secrets \
    && install -d -o concord -g concord -m 0755 /app /etc/concord

WORKDIR /app
COPY --from=server-builder --chown=concord:concord /build/target/release/concord-server /app/concord-server
COPY --from=server-builder --chown=concord:concord /build/target/release/concord_operator /app/concord-operator
COPY --from=web-builder --chown=concord:concord /build/web/dist /app/static
COPY --chown=root:root concord.example.toml /etc/concord/concord.example.toml

ENV CONCORD_CONFIG=/etc/concord/concord.toml
ENV RUST_LOG=info
USER concord:concord
VOLUME ["/var/lib/concord"]
EXPOSE 8080 6667
ENTRYPOINT ["/usr/bin/tini", "--", "/app/concord-server"]
CMD ["serve"]
