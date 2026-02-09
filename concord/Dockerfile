# Stage 1: Build the Rust server
FROM rust:1.84-bookworm AS server-builder
WORKDIR /build
COPY server/ server/
WORKDIR /build/server
RUN cargo build --release

# Stage 2: Build the React frontend
FROM node:22-bookworm-slim AS web-builder
WORKDIR /build
COPY web/ web/
WORKDIR /build/web
RUN npm ci && npm run build

# Stage 3: Runtime image
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy built artifacts
COPY --from=server-builder /build/server/target/release/concord-server /app/concord-server
COPY --from=web-builder /build/web/dist/ /app/static/

# Copy example config
COPY concord.example.toml /app/concord.example.toml

# Data directory for SQLite
RUN mkdir -p /app/data
VOLUME /app/data

ENV DATABASE_URL="sqlite:/app/data/concord.db?mode=rwc"
ENV RUST_LOG="info"

EXPOSE 8080 6667

CMD ["/app/concord-server"]
