# AGENTS.md

This guide is for agentic coding agents working in `sqlite-mcp-rs`.
Follow it as the operational default for this repository.

## Repository Profile

- Language: Rust (edition 2024)
- Crate type: single package (`sqlite-mcp-rs`)
- Runtime: Tokio multi-threaded runtime
- Transport/domain: MCP server over stdio with SQLite-backed tools
- Optional feature set: `vector`

## Rule Files Present in Repo

- Cursor rules (`.cursor/rules/`): not found
- Cursor single-file rules (`.cursorrules`): not found
- Copilot instructions (`.github/copilot-instructions.md`): not found

If these files are added later, merge their instructions into this guide and treat conflicts as explicit policy decisions.

## Environment Setup

- Rust toolchain: stable
- Node.js: required for inspector integration script (`npx @modelcontextprotocol/inspector`)
- Optional: Python 3 and `jq` for integration script assertions

## Build Commands

Use these from repository root:

```bash
cargo build
```

With vector feature enabled:

```bash
cargo build --features vector
```

Build release artifacts locally:

```bash
cargo build --release
```

## Formatting and Lint Commands

Format check:

```bash
cargo fmt --all -- --check
```

Apply formatting:

```bash
cargo fmt --all
```

Clippy (strict mode used by agents):

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Clippy (without optional vector feature):

```bash
cargo clippy --all-targets -- -D warnings
```

## Test Commands

Run full test suite:

```bash
cargo test
```

Run tests with vector feature:

```bash
cargo test --features vector
```

Run a single unit test by exact name:

```bash
cargo test test_name -- --exact --nocapture
```

Run tests in one module/file pattern:

```bash
cargo test policy::
```

Run one ignored test (repo example):

```bash
cargo test --features vector adapters::reranker::tests::downloads_and_uses_real_fastembed_models -- --ignored --exact --nocapture
```

List all discovered tests:

```bash
cargo test -- --list
```

## Integration Test Commands

Run MCP inspector integration flow:

```bash
bash scripts/test-sqlite-mcp-inspector.sh cargo run --
```

Run server directly:

```bash
cargo run
```

Run server with vector feature:

```bash
cargo run --features vector
```

## Code Style and Structure

### Imports

- Group imports in this order: std, third-party crates, local `crate::...` modules.
- Keep imports explicit; avoid wildcard imports.
- Use `cfg`-gated imports for optional feature code.

### Formatting

- Use `rustfmt` defaults (`cargo fmt --all`).
- Keep functions focused and short where practical.
- Prefer line breaks and helper functions over dense nested expressions.

### Types and API Contracts

- Prefer strong domain types and typed envelopes over untyped JSON blobs.
- Use `AppResult<T>` and `AppError` for fallible flows.
- Preserve MCP response envelope shape and metadata fields.
- Use feature gates (`#[cfg(feature = "vector")]`) instead of runtime feature flags.

### Naming Conventions

- Types/traits/enums: `PascalCase`.
- Functions/methods/modules/variables/tests: `snake_case`.
- Constants/statics: `SCREAMING_SNAKE_CASE`.
- Use descriptive names that encode domain meaning (`confirm_destructive`, `max_rows`, etc.).

### Error Handling

- Do not use `unwrap`/`expect` in production code paths.
- Return domain-specific `AppError` variants with actionable messages.
- Map internal errors to protocol errors through centralized conversion logic.
- Keep retryability and error code semantics stable.
- Fail fast on validation errors before doing I/O.

### Validation and Safety Boundaries

- Enforce SQL policy checks before execution:
  - max SQL length,
  - statement count/single-statement constraints,
  - blocked SQL (`ATTACH`, `LOAD_EXTENSION`),
  - protected table restrictions,
  - destructive operation confirmation gates.
- Preserve configured row/byte/db-size bounds.
- Keep path handling and persistence behavior canonicalized and validated.

### Concurrency and Blocking Work

- Keep server handlers thin and delegate domain logic to `tools::*`/`db::*` modules.
- Use `run_blocking` (or equivalent) when crossing from async handlers to blocking SQLite work.
- Share mutable state with `Arc<Mutex<...>>` only when needed; minimize lock scope.

### Logging and Observability

- Use `tracing` for structured runtime logs.
- Include request IDs for tool calls and error paths.
- Differentiate retryable vs non-retryable failures in logs.

### Testing Expectations for Changes

- Add/adjust unit tests near changed logic (`#[cfg(test)] mod tests`).
- Cover both success and failure paths, especially policy and error mapping paths.
- For SQL or DB lifecycle changes, include integration coverage via inspector script when feasible.

## Change Workflow for Agents

1. Read relevant modules and identify invariants (limits, policy checks, envelope contracts).
2. Implement minimal, coherent changes at the right abstraction layer.
3. Run format, lint, and targeted tests first; then broaden to full suite when possible.
4. Verify no regression in error mapping or tool response contract.
5. Keep feature-gated code complete for both enabled and disabled builds.

## High-Risk Areas (Handle Carefully)

- SQL policy scanning and protected table guards.
- Database size and persistence-path enforcement.
- Cursor pagination integrity and request fingerprinting.
- MCP error code mapping and retryable flags.
- Vector feature boundaries (`cfg`-gated compilation + runtime dependencies).

## Done Criteria for Agent Changes

- `cargo fmt --all` passes.
- `cargo clippy --all-targets --all-features -- -D warnings` passes (or scope-constrained equivalent with rationale).
- Relevant tests pass, including single-test reproductions for touched logic.
- Public behavior and safety guards remain consistent unless intentionally changed.
- Documentation/comments are updated only where needed to clarify non-obvious behavior.
