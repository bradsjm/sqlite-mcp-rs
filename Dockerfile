FROM rust:1-alpine AS builder
WORKDIR /app
RUN apk add --no-cache musl-dev
COPY . .
RUN cargo build --release

FROM scratch
LABEL org.opencontainers.image.description="Bounded SQLite MCP server over stdio with typed tool contracts, cursor-based pagination, and optional vector search"
COPY --from=builder /app/target/release/sqlite-mcp-rs /sqlite-mcp-rs
ENTRYPOINT ["/sqlite-mcp-rs"]
