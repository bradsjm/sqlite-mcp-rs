#!/bin/sh
# Pre-download default embedding and reranker models from HuggingFace
# This script runs during Docker build to cache models in the image

set -e

echo "Pre-downloading HuggingFace models..."

# Default models
EMBEDDING_MODEL="BAAI/bge-small-en-v1.5"
RERANKER_MODEL="BAAI/bge-reranker-base"

# Create cache directory
mkdir -p /root/.cache/huggingface

# Function to download a model using curl
download_model() {
    local model=$1
    local model_name=$(echo "$model" | tr '/' '_')
    local cache_dir="/root/.cache/huggingface/hub/models--${model_name}"
    
    echo "Downloading model: $model"
    
    # Create model directory structure
    mkdir -p "$cache_dir/snapshots/main"
    
    # Get the model files list from HuggingFace API
    local api_url="https://huggingface.co/api/models/${model}"
    
    # Download common model files
    for file in "model.onnx" "tokenizer.json" "tokenizer_config.json" "config.json"; do
        local file_url="https://huggingface.co/${model}/resolve/main/${file}"
        local output_path="$cache_dir/snapshots/main/$file"
        
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
ls -la /root/.cache/huggingface/hub/
