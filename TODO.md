# MCP Server Functionality Hardening Plan

This plan prioritizes high-risk functionality fixes first and encodes the decision that `sql_batch` is write-only.

## Priority Order

1. P0: abuse resistance and correctness guardrails
2. P1: lifecycle and data integrity fixes
3. P2: performance and observability improvements

---

## P0 - Immediate Fixes

### 1) Make `sql_batch` write-only (DECIDED)

Status: planned

Scope:
- Update `sql_batch` behavior to reject read-only statements.
- Keep `sql_query` as the only read path with pagination and response size limits.

Concrete changes:
- In `src/tools/sql.rs` inside `sql_batch`:
  - After preparing each statement, if `statement.readonly()` is true, return:
    - `AppError::InvalidInput("sql_batch only supports write statements; use sql_query for reads".to_string())`
  - Remove the current read branch that executes `raw_query()` and exhausts rows.
- Keep transaction logic as-is (`required` vs `none`).
- Keep existing destructive-confirmation behavior.

Tests:
- Add unit test: `sql_batch_rejects_read_statement`.
- Add unit test: mixed batch (`INSERT`, `SELECT`) fails at first read statement.
- Confirm existing write-only batch tests still pass.

Acceptance criteria:
- Any read statement in `sql_batch` returns `INVALID_PARAMS` mapped from `AppError::InvalidInput`.
- No code path in `sql_batch` executes `raw_query()`.

---

### 2) Bound vector workload inputs

Status: planned

Scope:
- Prevent unbounded `top_k` and `rerank_fetch_k` values.

Concrete changes:
- Add config fields in `src/config.rs`:
  - `max_vector_top_k: usize` (default 200)
  - `max_rerank_fetch_k: usize` (default 500)
- Parse env vars:
  - `SQLITE_MAX_VECTOR_TOP_K`
  - `SQLITE_MAX_RERANK_FETCH_K`
- Validate request values in `src/tools/vector.rs`:
  - reject `top_k == 0`
  - reject `top_k > policy/config cap`
  - reject `rerank_fetch_k < top_k` when rerank is enabled
  - reject `rerank_fetch_k > cap`

Tests:
- invalid `top_k=0` rejected
- oversized `top_k` rejected
- oversized `rerank_fetch_k` rejected
- valid bounded values succeed

Acceptance criteria:
- No large unbounded scan/rerank request can bypass configured limits.

---

### 3) Block internal vector metadata tampering from generic SQL tools

Status: planned

Scope:
- Prevent `sql_execute` and `sql_batch` from modifying vector system metadata.

Concrete changes:
- In SQL policy checks (`src/policy.rs` or `src/tools/sql.rs`), add a block list for write targets:
  - `_vector_collections`
- For now, reject any write statement containing `_vector_collections` token outside literals/comments.
- Return `AppError::PreconditionRequired` with explicit message.

Tests:
- `UPDATE _vector_collections ...` blocked
- `INSERT INTO _vector_collections ...` blocked
- ordinary writes to user tables still allowed

Acceptance criteria:
- Generic SQL tools cannot mutate vector control-table metadata.

---

### 4) Remove unsafe ORT env mutation from runtime request path

Status: planned

Scope:
- Avoid thread-unsafe `set_var` behavior after runtime threads are active.

Concrete changes:
- In `src/main.rs`, resolve ORT dylib path during startup before server service starts.
- Refactor `src/adapters/ort_runtime.rs`:
  - split into `resolve_ort_dylib_path(...) -> PathBuf`
  - set env var once during startup only
  - request-time code should not call `set_var`
- Guard with `OnceLock` for idempotent initialization.

Tests:
- startup path sets ORT env once
- repeated initialization calls are no-ops

Acceptance criteria:
- no `unsafe { std::env::set_var(...) }` in hot request paths.

---

## P1 - Functional Lifecycle and Integrity

### 5) Add `db_close` tool

Status: planned

Scope:
- Allow explicit connection lifecycle management.

Concrete changes:
- Add contracts in `src/contracts/db.rs`:
  - `DbCloseRequest { db_id: Option<String> }`
  - `DbCloseData { db_id: String, closed: bool, active_db_id: String }`
- Implement in `src/db/registry.rs`:
  - remove handle by id
  - if closing active handle, switch active id deterministically
- Implement tool in `src/tools/db.rs` and route in `src/server/mcp.rs`.
- Invalidate all cursors for closed db (`CursorStore::invalidate_db`).

Tests:
- close existing handle succeeds
- close unknown handle returns `NotFound`
- closing active handle updates active id

Acceptance criteria:
- clients can release DB resources without restart.

---

### 6) Enforce DB size limit during `db_open` for persisted DBs

Status: planned

Concrete changes:
- In `src/db/registry.rs`, after resolving persisted path and opening connection, enforce:
  - `enforce_db_size_limit(Some(path), max_db_bytes)`
- Wire `max_db_bytes` into `open_db` call path.

Tests:
- opening oversized persisted DB fails immediately
- opening compliant DB succeeds

Acceptance criteria:
- oversized DBs are rejected at open time, not only after writes.

---

### 7) Decouple persisted list limit from query row limit

Status: planned

Concrete changes:
- Add dedicated config:
  - `max_persisted_list_entries: usize` (default 500)
  - env `SQLITE_MAX_PERSISTED_LIST_ENTRIES`
- Use this in `db_list` instead of `max_rows`.

Tests:
- listing limit obeys dedicated config
- query row settings do not affect persisted list size

Acceptance criteria:
- predictable listing behavior independent of SQL query tuning.

---

### 8) Harden import identifier handling

Status: planned

Concrete changes:
- In `src/tools/import.rs`:
  - reject empty resolved columns with explicit `InvalidInput`
  - quote table and column identifiers safely for SQLite
  - keep identifier validation but no raw identifier interpolation

Tests:
- empty-column CSV/JSON import rejected with clear error
- keyword-like names handled safely via quoting

Acceptance criteria:
- import cannot generate malformed SQL due to identifier edge cases.

---

### 9) Eliminate `extension_flags` unused-variable warning

Status: planned

Concrete changes:
- In `src/db/registry.rs`, make `extension_flags` compile warning-free when `vector` is disabled:
  - either rename parameter to `_connection`
  - or split with cfg-gated signatures (`#[cfg(feature = "vector")]` uses `connection`, `#[cfg(not(feature = "vector"))]` has no parameter)
- Keep behavior unchanged.

Tests/verification:
- `cargo check`
- `cargo check --features vector`

Acceptance criteria:
- no `unused variable: connection` warning from `src/db/registry.rs` in default build.

---

## P2 - Performance and Observability

### 10) Reduce lock contention and blocking in async handlers

Status: planned

Concrete changes:
- Narrow lock scope in `src/server/mcp.rs`.
- Evaluate moving heavy rusqlite paths to `spawn_blocking` with clear ownership boundaries.
- Avoid holding both registry and cursor locks during long operations.

Tests/verification:
- concurrency stress test with simultaneous `sql_query` and `sql_execute`
- latency comparison before/after

Acceptance criteria:
- one slow request no longer serializes unrelated tool calls.

---

### 11) Cursor behavior cleanup

Status: planned

Concrete changes:
- Define explicit behavior for `cursor_capacity=0` (disable cursors entirely).
- Update `sql_query` to avoid emitting continuation cursor/hints when cursors are disabled.

Tests:
- capacity zero yields no stored/retrievable cursors
- truncated queries return no continuation cursor when disabled

Acceptance criteria:
- cursor semantics are explicit and consistent.

---

### 12) Improve error/log correlation

Status: planned

Concrete changes:
- Use one request ID per tool invocation across success/error logs and response metadata.
- Revisit MCP error code mapping granularity for better client behavior.

Tests:
- log assertions verify request ID consistency

Acceptance criteria:
- operational debugging can reliably correlate response and logs.

---

## Delivery Plan (PR Sequence)

1. PR1 (P0): `sql_batch` write-only + vector bound caps
2. PR2 (P0): vector metadata write protection + ORT startup refactor
3. PR3 (P1): `db_close` + `db_open` size-limit enforcement
4. PR4 (P1): persisted-list config decoupling + import hardening
5. PR5 (P1/P2): warning cleanup + concurrency/locking improvements + cursor cleanup + logging consistency

## Verification Matrix

- `cargo test`
- `cargo test --features vector`
- `cargo check`
- `cargo check --features vector`

## Notes

- No fallback/legacy behavior for `sql_batch` reads: this is a deliberate breaking change.
- Clients must use `sql_query` for all read statements.
