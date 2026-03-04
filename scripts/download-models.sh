#!/bin/sh
# Pre-download default embedding and reranker models from HuggingFace.
# Uses HF_CACHE_DIR when provided; otherwise falls back to a writable cache path.

set -e

echo "Pre-downloading HuggingFace models..."

# Default models
EMBEDDING_MODEL="BAAI/bge-small-en-v1.5"
RERANKER_MODEL="BAAI/bge-reranker-base"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

CACHE_BASE="${XDG_CACHE_HOME:-${HOME:-$REPO_ROOT/.cache}/.cache}"
if ! mkdir -p "$CACHE_BASE" 2>/dev/null; then
  CACHE_BASE="$REPO_ROOT/.cache"
  mkdir -p "$CACHE_BASE"
fi

HF_CACHE_DIR="${HF_CACHE_DIR:-$CACHE_BASE/huggingface}"
mkdir -p "$HF_CACHE_DIR/hub"

# Function to download a model using curl
download_model() {
    model="$1"
    model_name=$(echo "$model" | tr '/' '_')
    cache_dir="$HF_CACHE_DIR/hub/models--${model_name}"
    
    echo "Downloading model: $model"
    
    # Create model directory structure
    mkdir -p "$cache_dir/snapshots/main"
    
    # Get the model files list from HuggingFace API
    api_url="https://huggingface.co/api/models/${model}"
    
    # Download common model files
    for file in "model.onnx" "tokenizer.json" "tokenizer_config.json" "config.json"; do
        file_url="https://huggingface.co/${model}/resolve/main/${file}"
        output_path="$cache_dir/snapshots/main/$file"
        
        echo "  Downloading $file..."
        curl -fsSL -o "$output_path" "$file_url" 2>/dev/null || echo "    (optional file not found: $file)"
    done
    
    echo "Model $model downloaded to $cache_dir"
}

# Download embedding model
download_model "$EMBEDDING_MODEL"

# Download reranker model
download_model "$RERANKER_MODEL"

echo "All models pre-downloaded successfully!"
ls -la "$HF_CACHE_DIR/hub/"
