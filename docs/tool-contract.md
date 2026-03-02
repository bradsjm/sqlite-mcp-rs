# Tool Contract

This document defines the stable MCP tool surface for `sqlite-mcp-rs`.

## Tools

- `db_open`
- `db_list`
- `sql_query`
- `sql_execute`
- `sql_batch`
- `db_import`
- `vector_collection_create` (feature-gated: `vector`)
- `vector_collection_list` (feature-gated: `vector`)
- `vector_upsert` (feature-gated: `vector`)
- `vector_search` (feature-gated: `vector`)

## Common response envelope

All successful tool responses return:

- `summary: string`
- `data: object` (tool-specific payload)
- `hints?: ToolHint[]` (omitted when empty)
- `_meta.now_utc: string` (RFC3339 UTC)
- `_meta.elapsed_ms: number`
- `_meta.request_id: string` (UUID v4)
- `_meta.truncated?: boolean`
- `_meta.next_cursor?: string`

`ToolHint` shape:

- `tool: string`
- `arguments: object`
- `reason: string`

## `db_open`

Description: open an in-memory or persisted SQLite database and activate it.

Input:

- `db_id?: string` (default `"default"`)
- `mode: "memory" | "persist"`
- `path?: string`
  - Required when `mode = "persist"`.
  - Must stay inside `SQLITE_PERSIST_ROOT`.
  - Must not contain `..` path segments.
- `reset?: boolean` (default `false`)

Success `data` shape:

- `db_id: string`
- `mode: "memory" | "persist"`
- `path?: string`
- `active: true`
- `extensions_loaded.vec: boolean` (currently `false`)
- `extensions_loaded.rembed: boolean` (currently `false`)
- `extensions_loaded.regex: boolean` (currently `false`)

Validation and behavior:

- If the same `db_id` is already open with a different `(mode, path)` and `reset=false`, request fails.
- If `reset=true`, existing handle is replaced.
- `mode="persist"` requires `SQLITE_PERSIST_ROOT`.

## `db_list`

Description: list open and persisted database handles.

Input:

- `{}`

Success `data` shape:

- `active_db_id: string`
- `open: DbSummary[]`
- `persisted: string[]`
- `persisted_truncated: boolean`

`DbSummary` shape:

- `db_id: string`
- `mode: "memory" | "persist"`
- `path?: string`

Behavior:

- `persisted` is discovered from files under `SQLITE_PERSIST_ROOT` when configured.
- Persisted listing is capped by `SQLITE_MAX_ROWS`; overflow sets `persisted_truncated=true`.

## `sql_query`

Description: execute one read-only SQL statement with bounded row/byte output and cursor pagination.

Input:

- `db_id?: string` (default `"default"`)
- `sql?: string`
- `params?: SqlParams`
- `max_rows?: number` (default `SQLITE_MAX_ROWS`)
- `max_bytes?: number` (default `SQLITE_MAX_BYTES`)
- `cursor?: string`

`SqlParams`:

- positional: `Value[]`
- named: `{ [name: string]: Value }`
  - Names may be provided with or without `:`, `@`, `$` prefixes.

Cursor rules:

- If `cursor` is provided, `sql` and `params` must be omitted.
- Cursor resumes prior query context (`db_id`, SQL, params, limits, offset).
- Expired/unknown/invalid cursor fails with `NOT_FOUND`.

Success `data` shape:

- `columns: string[]`
- `rows: object[]`
- `row_count: number`
- `truncated: boolean`
- `next_cursor?: string`

Validation and enforcement:

- Exactly one SQL statement is required.
- Statement must be read-only.
- SQL length must be `<= SQLITE_MAX_SQL_LENGTH`.
- Blocked SQL is rejected:
  - `ATTACH ...`
  - `LOAD_EXTENSION(...)`
- `max_rows` and `max_bytes` must be `> 0`.
- Result truncation occurs when row count or byte budget is reached.
- On truncation, a continuation hint is emitted for `sql_query` with `cursor`.

## `sql_execute`

Description: execute one non-read SQL statement.

Input:

- `db_id?: string` (default `"default"`)
- `sql: string`
- `params?: SqlParams`

Success `data` shape:

- `rows_affected: number`
- `last_insert_rowid?: number` (set for `INSERT...` statements)

Validation and enforcement:

- Exactly one SQL statement is required.
- Statement must be non-read (`SELECT`-like statements rejected).
- SQL length must be `<= SQLITE_MAX_SQL_LENGTH`.
- Blocked SQL (`ATTACH`, `LOAD_EXTENSION(...)`) is rejected.
- For persisted DBs, post-write file size must be `<= SQLITE_MAX_DB_BYTES`.

## `sql_batch`

Description: execute multiple SQL statements, optionally in a transaction.

Input:

- `db_id?: string` (default `"default"`)
- `transaction: "required" | "none"`
- `confirm_destructive?: boolean` (default `false`)
- `statements: BatchStatement[]`

`BatchStatement`:

- `sql: string`
- `params?: SqlParams`

Success `data` shape:

- `transaction: "required" | "none"`
- `executed: number`
- `results: SqlBatchResult[]`

`SqlBatchResult`:

- `index: number`
- `kind: "query" | "execute"`
- `rows_affected: number`
- `last_insert_rowid?: number`

Validation and enforcement:

- At least one statement is required.
- `statements.len()` must be `<= SQLITE_MAX_STATEMENTS`.
- Every statement must be non-empty and contain exactly one SQL statement.
- SQL length per statement must be `<= SQLITE_MAX_SQL_LENGTH`.
- Blocked SQL (`ATTACH`, `LOAD_EXTENSION(...)`) is rejected.
- Destructive batch guard:
  - `DROP ...`
  - `TRUNCATE ...`
  - `DELETE FROM ...` without `WHERE`
  - requires `confirm_destructive=true`.
- If `transaction="required"`, any error triggers rollback.
- For persisted DBs, write statements enforce `SQLITE_MAX_DB_BYTES`.

## `db_import`

Description: import CSV or JSON rows into a table.

Input:

- `db_id?: string` (default `"default"`)
- `format: "csv" | "json"`
- `table: string` (must match `^[A-Za-z_][A-Za-z0-9_]*$`)
- `columns?: string[]`
- `data: ImportPayload`
- `batch_size?: number` (must be `> 0` when provided)
- `on_conflict?: "none" | "ignore" | "replace"`
- `truncate_first?: boolean` (default `false`)

`ImportPayload`:

- `string`
  - CSV: required, parsed as CSV text with headers.
  - JSON: parsed as JSON array of objects.
- `object[]`
  - JSON rows shorthand.

Success `data` shape:

- `table: string`
- `columns: string[]`
- `rows_inserted: number`
- `rows_skipped: number`

Validation and enforcement:

- Payload encoded size must be `<= SQLITE_MAX_BYTES`.
- Parsed row count must be `1..=SQLITE_MAX_ROWS`.
- Column names must match `^[A-Za-z_][A-Za-z0-9_]*$`.
- Runs in a transaction; failures rollback.
- For persisted DBs, post-import file size must be `<= SQLITE_MAX_DB_BYTES`.

## `vector_collection_create` (feature `vector`)

Description: create metadata and backing tables for a vector collection.

Input:

- `db_id?: string` (default `"default"`)
- `collection: string` (must match `^[A-Za-z_][A-Za-z0-9_]*$`)
- `dimension: number` (must be `> 0`)
- `if_not_exists?: boolean` (default `false`)

Success `data` shape:

- `collection: string`
- `docs_table: string` (`<collection>_docs`)
- `vec_table: string` (`<collection>_vec`)
- `created: boolean`

Validation and enforcement:

- Existing collection with `if_not_exists=false` returns `CONFLICT`.
- For persisted DBs, post-create file size must be `<= SQLITE_MAX_DB_BYTES`.

## `vector_collection_list` (feature `vector`)

Description: list vector collections.

Input:

- `db_id?: string` (default `"default"`)

Success `data` shape:

- `collections: VectorCollectionSummary[]`

`VectorCollectionSummary`:

- `collection: string`
- `docs_count: number`
- `dimension: number`
- `last_updated?: string`

Behavior:

- Returns an empty list when `_vector_collections` table does not exist.

## `vector_upsert` (feature `vector`)

Description: upsert vector documents into an existing collection.

Input:

- `db_id?: string` (default `"default"`)
- `collection: string`
- `on_conflict?: "replace" | "ignore" | "update_metadata"` (default `"replace"`)
- `items: VectorDocument[]` (must be non-empty)

`VectorDocument`:

- `id: string`
- `text: string`
- `metadata?: object`

Success `data` shape:

- `upserted_count: number`
- `skipped_count: number`

Validation and enforcement:

- Collection must exist.
- Embedding dimension must match collection dimension.
- Runs in a transaction; failures rollback.
- For persisted DBs, post-write file size must be `<= SQLITE_MAX_DB_BYTES`.

## `vector_search` (feature `vector`)

Description: semantic search over vector collections with optional reranking.

Input:

- `db_id?: string` (default `"default"`)
- `collection: string`
- `query_text: string`
- `top_k?: number` (default `10`, min effective `1`)
- `include_text?: boolean` (default `false`)
- `include_metadata?: boolean` (default `false`)
- `filter?: object` (exact metadata key/value match)
- `rerank?: "off" | "on"` (default `"off"`)
- `rerank_fetch_k?: number` (default `top_k`; clamped to at least `top_k`)

Success `data` shape:

- `matches: VectorMatch[]`
- `truncated: boolean`
- `reranked: boolean`
- `rerank_model?: string`
- `issues: VectorIssue[]`

`VectorMatch`:

- `id: string`
- `distance: number` (cosine distance; lower is better)
- `score?: number` (reranker score when reranked)
- `text?: string` (when `include_text=true`)
- `metadata?: object` (when `include_metadata=true`)

`VectorIssue`:

- `stage: string`
- `code: string`
- `message: string`
- `retryable: boolean`

Behavior:

- Base ranking uses cosine distance.
- Rows with malformed embeddings or dimension mismatch are skipped.
- `rerank="on"` without configured reranker does not fail the tool; it returns `issues` with `RERANK_UNAVAILABLE` and falls back to distance ranking.
- Reranker runtime failures do not fail the tool; it returns `issues` with `RERANK_FAILED` and falls back to distance ranking.

## Error code mapping

Tool errors are returned as MCP errors with:

- `error.code = INVALID_PARAMS` for domain/validation/dependency/database issues.
- `error.code = INTERNAL_ERROR` only for internal failures.
- `error.data.code` as one of:
  - `INVALID_INPUT`
  - `NOT_FOUND`
  - `CONFLICT`
  - `PRECONDITION_REQUIRED`
  - `FEATURE_DISABLED`
  - `CONFIG_MISSING`
  - `LIMIT_EXCEEDED`
  - `SQL_ERROR`
  - `DEPENDENCY_ERROR`
  - `INTERNAL`
- `error.data.retryable: boolean` (`true` for `DEPENDENCY_ERROR`; otherwise `false`).

## Environment and policy keys

Runtime configuration keys:

- `SQLITE_PERSIST_ROOT`
- `SQLITE_LOG_LEVEL`
- `SQLITE_MAX_SQL_LENGTH`
- `SQLITE_MAX_STATEMENTS`
- `SQLITE_MAX_ROWS`
- `SQLITE_MAX_BYTES`
- `SQLITE_MAX_DB_BYTES`
- `SQLITE_CURSOR_TTL_SECONDS`
- `SQLITE_CURSOR_CAPACITY`

Vector feature keys (required when `vector` is enabled unless noted):

- `SQLITE_EMBEDDING_PROVIDER` (`builtin`)
- `SQLITE_EMBEDDING_MODEL`
- `SQLITE_EMBEDDING_ENDPOINT` (optional)
- `SQLITE_EMBEDDING_SIZE`
- `SQLITE_RERANKER_PROVIDER` (optional, but required if any reranker key is set; `builtin`)
- `SQLITE_RERANKER_MODEL` (optional, but required if reranker is configured)
- `SQLITE_RERANKER_ENDPOINT` (optional)
- `SQLITE_RERANKER_TIMEOUT_MS` (optional; default `10000`)
