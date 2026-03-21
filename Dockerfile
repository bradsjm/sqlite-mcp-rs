ARG DEBIAN_VERSION=bookworm
ARG RUST_VERSION=1.94

FROM rust:${RUST_VERSION}-${DEBIAN_VERSION} AS builder
WORKDIR /app
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      build-essential \
      clang \
      curl \
      libclang-dev \
      libssl-dev \
      pkg-config \
      python3 \
      tar \
      gzip && \
    rm -rf /var/lib/apt/lists/*
ARG CARGO_FEATURES="vector local-embeddings"
COPY . .
RUN cargo build --release --features "$CARGO_FEATURES"

FROM builder AS artifact
RUN cp /app/target/release/sqlite-mcp-rs /sqlite-mcp-rs

FROM builder AS runtime-deps
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

# Pre-download HuggingFace models
RUN chmod +x scripts/download-models.sh && ./scripts/download-models.sh

FROM debian:${DEBIAN_VERSION}-slim
LABEL org.opencontainers.image.description="Bounded SQLite MCP server over stdio with typed tool contracts, cursor-based pagination, and optional vector search"
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libgcc-s1 libstdc++6 && \
    rm -rf /var/lib/apt/lists/*
COPY --from=runtime-deps /app/target/release/sqlite-mcp-rs /sqlite-mcp-rs
COPY --from=runtime-deps /opt/onnxruntime /opt/onnxruntime
# Copy pre-downloaded models from builder
COPY --from=runtime-deps /root/.cache/huggingface /root/.cache/huggingface
ENV ORT_DYLIB_PATH=/opt/onnxruntime/lib/libonnxruntime.so
# Set HuggingFace cache directory
ENV HF_HOME=/root/.cache/huggingface
ENTRYPOINT ["/sqlite-mcp-rs"]
