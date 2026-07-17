# Release Guide

This document defines the release system for `sqlite-mcp-rs`.

## Goals

The current release design is optimized for these requirements:

- manual releases only
- one source of truth for the release version: `[package].version` in `Cargo.toml`
- one user-facing npm package name
- native binaries for macOS, Windows, and Linux
- explicit GNU and musl Linux npm payloads selected by runtime libc
- Linux container images for `amd64` and `arm64`
- simple packaging logic that is checked into the repository and debuggable without generated installer code

## Platform Matrix

Release workflow binaries:

| Platform | Rust target | Delivery | Features |
| --- | --- | --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` | GitHub release archive, npm | `vector local-embeddings` |
| macOS Intel | `x86_64-apple-darwin` | GitHub release archive, npm | `vector local-embeddings` |
| Windows x64 | `x86_64-pc-windows-msvc` | GitHub release archive, npm | `vector local-embeddings` |
| Linux GNU x64 | `x86_64-unknown-linux-gnu` | GitHub release archive, npm | `vector local-embeddings` |
| Linux GNU arm64 | `aarch64-unknown-linux-gnu` | GitHub release archive, npm | `vector local-embeddings` |
| Linux musl x64 | `x86_64-unknown-linux-musl` | GitHub release archive, npm | `vector` |
| Linux musl arm64 | `aarch64-unknown-linux-musl` | GitHub release archive, npm | `vector` |

Docker workflow images:

| Platform | Delivery |
| --- | --- |
| `linux/amd64` | GHCR manifest image |
| `linux/arm64` | GHCR manifest image |

## Design Rules

These rules are the important part to copy into other MCP repositories.

### 1. Keep releases manual

Both publishing workflows use `workflow_dispatch` only:

- `.github/workflows/release.yml`
- `.github/workflows/publish-docker.yml`

This keeps release state explicit and avoids accidental publishes from tags or branch pushes.

### 2. Derive the version once from `Cargo.toml`

Both workflows parse `[package].version` from `Cargo.toml` with the same `awk` logic and derive:

- `version`: raw crate version, for example `0.2.0`
- `tag`: git-style release tag, for example `v0.2.0`

Do not add a second version source in workflow inputs, npm metadata, or Docker tags. That creates drift.

### 3. Use checked-in packaging code, not generated installers

This repository does not rely on generated npm or shell installers. Packaging behavior is defined in source:

- `scripts/package_release.py`
- `scripts/build_npm_packages.py`
- `scripts/write_checksums.py`
- `npm/meta/package.json`
- `npm/meta/bin/sqlite-mcp-rs.js`
- `npm/meta/lib/platform.js`

### 4. Publish one real npm package name

The npm release model uses:

- one real package name: `@bradsjm/sqlite-mcp-rs`
- one meta package at version `X.Y.Z`
- seven platform payload packages published under the same real package name with platform-suffixed versions such as `X.Y.Z-linux-x64-gnu`
- `optionalDependencies` aliases in the meta package that map friendly alias names back to the real package and version

In this repository the alias names are:

- `@bradsjm/sqlite-mcp-rs-darwin-x64`
- `@bradsjm/sqlite-mcp-rs-darwin-arm64`
- `@bradsjm/sqlite-mcp-rs-win32-x64`
- `@bradsjm/sqlite-mcp-rs-linux-x64-gnu`
- `@bradsjm/sqlite-mcp-rs-linux-arm64-gnu`
- `@bradsjm/sqlite-mcp-rs-linux-x64-musl`
- `@bradsjm/sqlite-mcp-rs-linux-arm64-musl`

These alias names are not independent packages that need separate npm initialization.

### 5. Select Linux payloads by libc

Linux payloads declare npm `os`, `cpu`, and `libc` metadata. The launcher independently detects libc from the Node.js diagnostic report, validates the installed payload metadata, and selects:

- GNU x64: `x86_64-unknown-linux-gnu`
- GNU arm64: `aarch64-unknown-linux-gnu`
- musl x64: `x86_64-unknown-linux-musl`
- musl arm64: `aarch64-unknown-linux-musl`

GNU artifacts are built with `vector local-embeddings` against an explicit glibc 2.28 floor. Musl artifacts retain the `vector`-only feature set. Detection fails closed if diagnostic reports are unavailable, ambiguous, or contain no recognized libc marker; the launcher never falls back across libc families.

Docker images remain separate glibc artifacts built with `vector local-embeddings`.

### 6. Keep Docker publishing separate from release artifact builds

The release workflow builds all GitHub release and npm payloads directly on CI runners. The Docker publish workflow is separate and is the only workflow that builds and publishes container images.

That keeps `release.yml` focused on release artifacts and keeps Docker concerns isolated in `publish-docker.yml`.

## Files That Define the Release System

| File | Responsibility |
| --- | --- |
| `.github/workflows/release.yml` | builds release archives, builds npm tarballs, runs smoke tests, creates GitHub release, publishes npm |
| `.github/workflows/publish-docker.yml` | builds and publishes multi-arch GHCR images |
| `.github/workflows/init-npm-placeholder.yml` | creates the initial npm package so Trusted Publishing can be configured |
| `Dockerfile` | multi-stage Docker image build used by Docker publishing |
| `scripts/package_release.py` | packages release archives and per-file checksums |
| `scripts/build_npm_packages.py` | stages platform npm tarballs and the meta tarball |
| `scripts/run-sqlite-mcp-glibc-baseline.sh` | runs release binaries in Rocky Linux 8 for glibc-floor integration tests |
| `scripts/test_build_npm_packages.py` | validates generated manifests and npm libc install filtering |
| `scripts/test-npm-launcher.js` | validates runtime platform/libc selection and payload metadata checks |
| `scripts/write_checksums.py` | writes `.sha256` files and the aggregate `sha256.sum` |
| `npm/meta/package.json` | base metadata for the npm meta package |
| `npm/meta/bin/sqlite-mcp-rs.js` | runtime launcher that resolves the installed platform payload |
| `npm/meta/lib/platform.js` | detects libc, resolves targets, and validates payload package metadata |

## Release Workflow

Source: `.github/workflows/release.yml`

### Job order

1. `version`
2. `validate`
3. `build-release-artifacts`
4. `package-npm`
5. `smoke-npm-native`, `smoke-npm-musl`, and `smoke-gnu-model`
6. `publish`

### `version`

Purpose:

- checkout the repository
- read `Cargo.toml`
- export `version` and `tag`

This job must remain small and deterministic because every publish step depends on it.

### `validate`

Purpose:

- `node scripts/test-npm-launcher.js`
- `python scripts/test_build_npm_packages.py` with Node 24 and npm 12.0.1
- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- default, vector, and all-feature `cargo test --locked` runs

This is the gate that prevents packaging broken code.

### `build-release-artifacts`

Purpose:

- build every release binary in one matrix job
- use `cargo build --locked --release --features "vector local-embeddings" --target ...` for macOS and Windows targets
- use `cargo zigbuild --locked --release` for Linux targets
- run direct `--help` smoke tests on matching-architecture runners
- run every GNU binary in Rocky Linux 8 before packaging
- package each binary with `scripts/package_release.py`
- upload packaged archives as workflow artifacts

Current matrix:

- `aarch64-apple-darwin` on `macos-15`
- `x86_64-apple-darwin` on `macos-15-intel`
- `x86_64-pc-windows-msvc` on `windows-2022`
- `x86_64-unknown-linux-gnu.2.28` on `ubuntu-24.04`
- `aarch64-unknown-linux-gnu.2.28` on `ubuntu-24.04-arm`
- `x86_64-unknown-linux-musl` on `ubuntu-24.04`
- `aarch64-unknown-linux-musl` on `ubuntu-24.04-arm`

Linux implementation details:

- Linux jobs install Zig and `cargo-zigbuild`.
- Archive names and Cargo output paths use canonical Rust triples without the `.2.28` build suffix.
- GNU artifacts use `vector local-embeddings`; musl artifacts use `vector`.
- Linux jobs keep the compatibility `CFLAGS` needed by `sqlite-vec`.

Current limitation from source:

- Windows arm64 is not part of the release matrix or the smoke matrix.

### `package-npm`

Purpose:

- download all packaged release archives
- build npm tarballs from those archives with `scripts/build_npm_packages.py`
- upload the resulting `npm-dist` directory as a workflow artifact

Important design detail:

- npm packaging consumes the release archives, not `target/` directly

That keeps npm and GitHub release payloads consistent.

### Npm smoke gates

`smoke-npm-native` installs the meta tarball, manually extracts the matching payload, and runs `npx --no-install sqlite-mcp-rs --help` on macOS arm64/x64, Windows x64, and GNU Linux arm64/x64. Linux rows install both same-CPU libc payloads and assert the real runtime selects GNU.

`smoke-npm-musl` repeats the Linux install with both payloads inside matching-architecture `node:24-alpine` containers and asserts the runtime selects musl.

`smoke-gnu-model` extracts the exact GNU npm payload on both Linux architectures and runs the existing local-model inspector suite through Rocky Linux 8. Publication remains blocked until ONNX Runtime, embeddings, similarity search, and reranking succeed at the glibc floor.

Windows arm64 is not part of the release or smoke matrix.

### `publish`

Purpose:

- download all release archives and npm tarballs
- write unified release checksums with `scripts/write_checksums.py`
- verify all expected release files exist
- write release notes
- publish npm tarballs
- create the GitHub release with `gh release create`

Publish order is intentional:

1. publish all platform tarballs first
2. publish the meta tarball last
3. create the GitHub release after npm publishing succeeds

The meta package should not be visible before its platform variants exist, otherwise npm installs can resolve an incomplete release.

The npm publish step is also rerunnable. Before each `npm publish`, the workflow checks whether that exact package version already exists and skips it if it does. This allows a rerun to continue past previously published platform tarballs after a partial npm failure.

## Docker Publish Workflow

Source: `.github/workflows/publish-docker.yml`

### Job order

1. `meta`
2. `build-amd64`
3. `build-arm64`
4. `manifest`

### `meta`

Purpose:

- checkout repository
- resolve `v{version}` from `Cargo.toml`
- use `docker/metadata-action` to generate image tags

Current tags:

- `v{Cargo.toml version}`
- `latest`

### `build-amd64` and `build-arm64`

Purpose:

- login to GHCR with `GITHUB_TOKEN`
- build the Docker image for one Linux architecture
- push by digest
- smoke test the pushed digest with `docker run ... --help`

The jobs build separately so the final manifest can combine stable digests.

### `manifest`

Purpose:

- read the digests produced by the architecture-specific jobs
- create manifest lists for every resolved tag with `docker buildx imagetools create`

This is what makes `ghcr.io/<repo>:latest` and `ghcr.io/<repo>:vX.Y.Z` multi-arch images instead of single-arch tags.

## NPM Placeholder Workflow

Source: `.github/workflows/init-npm-placeholder.yml`

npm Trusted Publishing requires the package to exist before the trust relationship can be configured.

This workflow:

- checks whether `@<owner>/sqlite-mcp-rs` already exists
- if not, publishes a `0.0.0` placeholder using `NPM_TOKEN`
- leaves Trusted Publishing setup to npm afterwards

Important rule:

- initialize only the real package name

Do not initialize the alias package names. They are alias specifiers in `optionalDependencies`, not standalone npm packages.

## Packaging Scripts

### `scripts/package_release.py`

Responsibilities:

- package one built binary into a release archive
- include `README.md` and `LICENSE`
- emit `.zip` for Windows targets
- emit `.tar.xz` for non-Windows targets
- write a matching `<artifact>.sha256`

This script is intentionally small. Keep it that way.

### `scripts/build_npm_packages.py`

Responsibilities:

- define the supported npm platform matrix
- extract binaries from release archives into `vendor/<target-triple>/`
- build one tarball per platform package version
- build the meta tarball
- emit `npm-dist/manifest.json`

Important behavior:

- platform tarballs use the real npm package name and platform-suffixed versions
- the meta tarball writes `optionalDependencies` aliases that point back to the real package name
- Linux platform manifests include the npm `libc` array; Darwin and Windows manifests omit it

If another repo copies this pattern, update:

- package scope/name
- repository URLs
- platform alias names
- artifact filenames
- binary filename if it differs from the crate name

### `scripts/write_checksums.py`

Responsibilities:

- compute SHA256 for every release artifact
- write per-file `.sha256`
- write aggregate `sha256.sum`

The `publish` job uses this after all archives have been collected into `release/`.

## NPM Launcher Contract

Sources:

- `npm/meta/lib/platform.js`
- `npm/meta/bin/sqlite-mcp-rs.js`

The launcher:

1. maps `process.platform`, `process.arch`, and Linux libc to a Rust target triple
2. maps that target triple to an alias package name
3. resolves `<alias>/package.json`
4. validates the installed package `os`, `cpu`, and Linux `libc` metadata
5. runs the vendored binary from `vendor/<target-triple>/`

Supported runtime mappings:

| Runtime | Target triple | Alias package |
| --- | --- | --- |
| Linux GNU x64 | `x86_64-unknown-linux-gnu` | `@bradsjm/sqlite-mcp-rs-linux-x64-gnu` |
| Linux GNU arm64 | `aarch64-unknown-linux-gnu` | `@bradsjm/sqlite-mcp-rs-linux-arm64-gnu` |
| Linux musl x64 | `x86_64-unknown-linux-musl` | `@bradsjm/sqlite-mcp-rs-linux-x64-musl` |
| Linux musl arm64 | `aarch64-unknown-linux-musl` | `@bradsjm/sqlite-mcp-rs-linux-arm64-musl` |
| macOS x64 | `x86_64-apple-darwin` | `@bradsjm/sqlite-mcp-rs-darwin-x64` |
| macOS arm64 | `aarch64-apple-darwin` | `@bradsjm/sqlite-mcp-rs-darwin-arm64` |
| Windows x64 | `x86_64-pc-windows-msvc` | `@bradsjm/sqlite-mcp-rs-win32-x64` |

The launcher fails fast for unsupported platforms, unknown/ambiguous libc reports, missing selected payloads, and payload metadata mismatches.

## Lessons Learned

These points are why the current design looks the way it does.

### Avoid generated installer metadata

Generated installer metadata was the source of the earlier Alpine and Linux ABI issues. Checked-in packaging code is easier to inspect, patch, and reuse.

### Do not maintain multiple version sources

The release version must come from `Cargo.toml`. Npm and Docker should consume that version, not invent their own.

### Publish the meta npm package last

Publishing the meta package first creates a broken installation window. The workflow avoids that on purpose.

### Keep Linux npm selection explicit

GNU and musl payloads carry distinct identities. Npm metadata filters installs when supported, while the launcher validates libc at runtime so older npm behavior cannot cause a cross-libc fallback.

### Keep release and container workflows separate

Using direct CI builds for release artifacts and reserving Docker for the Docker publish workflow keeps the release pipeline easier to reason about.

### Smoke test the actual install path

The npm smoke jobs install the packed tarballs and execute the launcher. That is better than only checking that tarballs exist.

## Local Validation

The repository can be partially validated locally with `act` and Docker or Podman.

Useful local checks:

- `act -n workflow_dispatch -W .github/workflows/release.yml`
- `act -n workflow_dispatch -W .github/workflows/publish-docker.yml`
- `node scripts/test-npm-launcher.js`
- `uv run --no-project --python 3.12 scripts/test_build_npm_packages.py` with npm 12.0.1 on `PATH`
- GNU `cargo zigbuild` with the `.2.28` target suffix followed by Rocky Linux 8 `--help`
- the local inspector suite through `scripts/run-sqlite-mcp-glibc-baseline.sh`
- local GNU and musl tarball smoke tests with both same-CPU aliases installed
- local Docker build and `--help` smoke tests for `publish-docker.yml`

Known limitation from the current workflow shape:

- the full `release.yml` DAG cannot be executed end-to-end on a single Linux host with `act` because macOS and Windows jobs are part of the dependency chain

That is acceptable for this repository. The local goal is early structural validation, not full hosted-runner emulation.

## How To Reuse This In Another MCP Repository

If another Rust MCP repository wants the same release system, copy the structure and then change only the repository-specific values.

Required updates:

1. rename the package and binary in:
   - `Cargo.toml`
   - `.github/workflows/release.yml`
   - `scripts/package_release.py` invocation sites
   - `scripts/build_npm_packages.py`
   - `npm/meta/package.json`
   - `npm/meta/bin/<binary>.js`
2. update npm scope and repository URLs in:
   - `npm/meta/package.json`
   - `scripts/build_npm_packages.py`
   - `.github/workflows/init-npm-placeholder.yml`
3. update Docker image naming only through:
   - repository location
   - `.github/workflows/publish-docker.yml`
4. keep manual-only triggers
5. keep version derivation from `Cargo.toml`
6. keep platform payload publish order: platform tarballs first, meta tarball last
7. keep Docker publishing isolated to `.github/workflows/publish-docker.yml`

Do not copy old cargo-dist based installer generation into the new repository. That reintroduces the class of problems this design removed.
