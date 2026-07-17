#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [[ "$#" -lt 1 ]]; then
  echo "usage: scripts/run-sqlite-mcp-glibc-baseline.sh <absolute-binary> [args...]" >&2
  exit 2
fi

binary="$1"
shift

if [[ "$binary" != /* ]]; then
  echo "binary path must be absolute: $binary" >&2
  exit 2
fi

if [[ ! -x "$binary" ]]; then
  echo "binary is not executable: $binary" >&2
  exit 2
fi

binary="$(realpath "$binary")"
case "$binary" in
  "$REPO_ROOT"/*) ;;
  *)
    echo "binary must be under the repository root: $binary" >&2
    exit 2
    ;;
esac

exec docker run --rm -i \
  --user "$(id -u):$(id -g)" \
  --volume "$REPO_ROOT:$REPO_ROOT" \
  --workdir "$PWD" \
  -e SQLITE_PERSIST_ROOT \
  -e SQLITE_LOG_LEVEL \
  -e SQLITE_MAX_SQL_LENGTH \
  -e SQLITE_MAX_STATEMENTS \
  -e SQLITE_MAX_ROWS \
  -e SQLITE_MAX_BYTES \
  -e SQLITE_MAX_DB_BYTES \
  -e SQLITE_CURSOR_TTL_SECONDS \
  -e SQLITE_CURSOR_CAPACITY \
  -e SQLITE_INSPECTOR_DB_PATH \
  -e SQLITE_RERANKER_PROVIDER \
  -e SQLITE_RERANKER_MODEL \
  -e SQLITE_EMBEDDING_CACHE_DIR \
  -e SQLITE_RERANKER_CACHE_DIR \
  rockylinux:8 \
  "$binary" "$@"
