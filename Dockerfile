FROM rust:1-alpine AS builder
WORKDIR /app
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig linux-headers curl tar gzip
ARG CARGO_FEATURES="vector"
COPY . .
# Fix: Define missing BSD types for musl compatibility (required by sqlite-vec)
ENV CFLAGS="-Du_int8_t=uint8_t -Du_int16_t=uint16_t -Du_int64_t=uint64_t"
RUN cargo build --release --features "$CARGO_FEATURES"

# Download ONNX Runtime for both architectures
ARG TARGETARCH
RUN mkdir -p /opt/onnxruntime && \
    if [ "$TARGETARCH" = "arm64" ]; then \
        curl -L -o /tmp/onnxruntime.tgz "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-aarch64-1.23.2.tgz"; \
    else \
        curl -L -o /tmp/onnxruntime.tgz "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-x64-1.23.2.tgz"; \
    fi && \
    tar -xzf /tmp/onnxruntime.tgz -C /opt/onnxruntime --strip-components=1 && \
    rm /tmp/onnxruntime.tgz

FROM alpine:latest
LABEL org.opencontainers.image.description="Bounded SQLite MCP server over stdio with typed tool contracts, cursor-based pagination, and optional vector search"
# Install CA certificates for HTTPS (required for HuggingFace model downloads)
RUN apk add --no-cache ca-certificates
COPY --from=builder /app/target/release/sqlite-mcp-rs /sqlite-mcp-rs
COPY --from=builder /opt/onnxruntime /opt/onnxruntime
ENV ORT_DYLIB_PATH=/opt/onnxruntime/lib/libonnxruntime.so
ENTRYPOINT ["/sqlite-mcp-rs"]
