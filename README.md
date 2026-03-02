# sqlite-mcp-rs

A bounded Model Context Protocol (MCP) server for SQLite with stdio (default) and optional HTTP transport. It supports both ephemeral in-memory databases and persisted databases, and provides typed tools for database lifecycle, query/execute operations, batch writes, data import, cursor-based pagination, and optional vector search.

## Features

- **Bounded SQL execution**: strict limits for statement length, row count, and response bytes
- **Safe-by-default policy**: blocks `ATTACH` and `LOAD_EXTENSION`, confirms destructive batch operations
- **Typed MCP contracts**: structured request/response schemas and consistent tool envelopes
- **Cursor pagination**: resumable `sql_query` with TTL and bounded cursor capacity
- **Ephemeral + persisted support**: open/list/close in-memory (ephemeral) and file-backed (persisted) SQLite databases
- **Import support**: ingest CSV or JSON rows into validated table/column targets
- **Lightweight queue primitives**: push JSON jobs and long-poll for new rows over MCP/HTTP
- **Optional vector search**: sqlite-vec collections with embedding and optional reranking (`vector` feature)

## Installation

Choose one of the following based on your environment.

### Using NPX

If this package is published to npm:

```bash
npx @bradsjm/sqlite-mcp-rs@latest
```

Or install globally:

```bash
npm install -g @bradsjm/sqlite-mcp-rs
sqlite-mcp-rs
```

### Using Docker

Local stdio transport with latest image:

```bash
docker run --rm -i --env-file .env ghcr.io/bradsjm/sqlite-mcp-rs:latest
```

Run over HTTP transport:

```bash
docker run --rm -i --env-file .env ghcr.io/bradsjm/sqlite-mcp-rs:latest -- --transport http --host localhost --port 3000
```

Build locally:

```bash
docker build -t sqlite-mcp-rs .
docker run --rm -i --env-file .env sqlite-mcp-rs
```

### From Source

```bash
cargo install --path .
```

Or run directly:

```bash
cargo run
```

Run over HTTP transport:

```bash
cargo run -- --transport http --host localhost --port 3000
```

## Quick Start

### Configure MCP

Use this MCP configuration template and set the **optional** environment value for persisted databases.

```json
{
  "mcpServers": {
    "sqlite": {
      "command": "npx",
      "args": ["-y", "@bradsjm/sqlite-mcp-rs@latest"],
      "env": {
        "SQLITE_PERSIST_ROOT": "/absolute/path/to/sqlite-data",
      }
    }
  }
}
```

### Transport Options

The server supports:

- `--transport stdio` (default)
- `--transport http --host localhost --port 3000`

When HTTP transport is enabled, expose and connect to:

```text
http://<host>:<port>/mcp
```

HTTP connections are not isolated by session for database handles. All HTTP clients in one server process share the same `SQLITE_PERSIST_ROOT` and the same in-process registry, so they can open and use the same persisted database file at the same time.

### Core Environment Variables

Defaults are shown below.

```bash
SQLITE_PERSIST_ROOT=                       # optional; if unset, only ephemeral (memory) databases are allowed
SQLITE_LOG_LEVEL=info
SQLITE_MAX_SQL_LENGTH=20000
SQLITE_MAX_STATEMENTS=50
SQLITE_MAX_ROWS=500
SQLITE_MAX_BYTES=1048576
SQLITE_MAX_DB_BYTES=100000000
SQLITE_MAX_PERSISTED_LIST_ENTRIES=500
SQLITE_CURSOR_TTL_SECONDS=600
SQLITE_CURSOR_CAPACITY=500
SQLITE_QUEUE_WAIT_TIMEOUT_MS_DEFAULT=30000
SQLITE_QUEUE_WAIT_TIMEOUT_MS_MAX=120000
SQLITE_QUEUE_POLL_INTERVAL_MS_DEFAULT=250
SQLITE_QUEUE_POLL_INTERVAL_MS_MIN=50
SQLITE_QUEUE_POLL_INTERVAL_MS_MAX=5000
```

### Vector Feature (Optional)

Build/run with vector support:

```bash
cargo run --features vector
```

Vector environment variables:

```bash
SQLITE_MAX_VECTOR_TOP_K=200
SQLITE_MAX_RERANK_FETCH_K=500
SQLITE_EMBEDDING_PROVIDER=fastembed
SQLITE_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5
SQLITE_EMBEDDING_CACHE_DIR=                # optional
SQLITE_RERANKER_PROVIDER=fastembed         # optional (only if reranker enabled)
SQLITE_RERANKER_MODEL=BAAI/bge-reranker-base  # optional (only if reranker enabled)
SQLITE_RERANKER_CACHE_DIR=                 # optional
```

## Tool Reference

All tools return a consistent envelope:

```json
{
  "summary": "Human-readable outcome",
  "data": {},
  "_meta": {
    "now_utc": "2026-03-02T00:00:00Z",
    "elapsed_ms": 12,
    "request_id": "uuid-v4"
  }
}
```

### Database Tools

| Tool | Purpose |
|------|---------|
| `db_open` | Open and activate memory or persisted database handle |
| `db_list` | List active/open handles and discovered persisted databases |
| `db_close` | Close a database handle and invalidate related cursors |

### SQL Tools

| Tool | Purpose |
|------|---------|
| `sql_query` | Execute one read-only statement with bounded results and optional cursor continuation |
| `sql_execute` | Execute one non-read statement and return write metadata |
| `sql_batch` | Execute multiple write statements with optional transaction and destructive guard |
| `db_import` | Import CSV/JSON rows into a table |

### Queue Tools

| Tool | Purpose |
|------|---------|
| `queue_push` | Insert a JSON job into a named queue |
| `queue_wait` | Long-poll for the next visible job after a caller baseline |

### Vector Tools (`vector` feature)

| Tool | Purpose |
|------|---------|
| `vector_collection_create` | Create vector collection backing tables |
| `vector_collection_list` | List vector collections and metadata |
| `vector_upsert` | Upsert embedded vector documents |
| `vector_search` | Run semantic KNN search with optional reranking |

For full schemas and validation rules, see `docs/tool-contract.md`.

## Policy and Safety

- `sql_query` accepts exactly one read-only statement.
- `sql_execute` and `sql_batch` reject read statements.
- `ATTACH` and `LOAD_EXTENSION` are blocked.
- Destructive batch writes require `confirm_destructive=true`.
- Writes to internal table `_vector_collections` are blocked from generic SQL tools.
- `queue_wait` defaults to `include_existing=false` so callers wait for new rows unless they opt in.

## Troubleshooting

### Persisted mode rejected

If `db_open` with `mode="persist"` fails, ensure `SQLITE_PERSIST_ROOT` is set.

### Query blocked by policy

If you see blocked SQL errors, remove `ATTACH` / `LOAD_EXTENSION` or use allowed statements.

### Batch rejected as destructive

For `DROP`, `TRUNCATE`, or `DELETE` without `WHERE`, set `confirm_destructive=true`.

### Vector tools unavailable

Run with `--features vector` and configure embedding env vars.

## Development

Run unit tests:

```bash
cargo test
```

Run integration checks against MCP Inspector:

```bash
bash scripts/test-sqlite-mcp-inspector.sh cargo run --
```

Run integration checks against MCP Inspector over HTTP transport:

```bash
bash scripts/test-sqlite-mcp-inspector-http.sh cargo run --
```

Show CLI help:

```bash
sqlite-mcp-rs --help
```
