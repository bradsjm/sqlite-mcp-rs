# Release Guide

This document defines the release system for `sqlite-mcp-rs`.

## Goals

The current release design is optimized for these requirements:

- manual releases only
- one source of truth for the release version: `[package].version` in `Cargo.toml`
- one user-facing npm package name
- native binaries for macOS, Windows, and Linux
- Linux npm support without a glibc/musl split in the installer
- Linux container images for `amd64` and `arm64`
- simple packaging logic that is checked into the repository and debuggable without generated installer code

## Platform Matrix

Release workflow binaries:

| Platform | Rust target | Delivery |
| --- | --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` | GitHub release archive, npm |
| macOS Intel | `x86_64-apple-darwin` | GitHub release archive, npm |
| Windows x64 | `x86_64-pc-windows-msvc` | GitHub release archive, npm |
| Windows arm64 | `aarch64-pc-windows-msvc` | GitHub release archive, npm |
| Linux x64 | `x86_64-unknown-linux-musl` | GitHub release archive, npm, Docker |
| Linux arm64 | `aarch64-unknown-linux-musl` | GitHub release archive, npm, Docker |

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

### 4. Publish one real npm package name

The npm release model uses:

- one real package name: `@bradsjm/sqlite-mcp-rs`
- one meta package at version `X.Y.Z`
- several platform payload packages published under the same real package name with platform-suffixed versions such as `X.Y.Z-linux-x64`
- `optionalDependencies` aliases in the meta package that map friendly alias names back to the real package and version

In this repository the alias names are:

- `@bradsjm/sqlite-mcp-rs-darwin-x64`
- `@bradsjm/sqlite-mcp-rs-darwin-arm64`
- `@bradsjm/sqlite-mcp-rs-win32-x64`
- `@bradsjm/sqlite-mcp-rs-win32-arm64`
- `@bradsjm/sqlite-mcp-rs-linux-x64`
- `@bradsjm/sqlite-mcp-rs-linux-arm64`

These alias names are not independent packages that need separate npm initialization.

### 5. Keep Linux npm packages musl-only

The npm launcher always selects the musl Linux builds:

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`

This avoids glibc baseline management in the npm path. This is a packaging decision, not a general rule for all Rust binaries. If a future MCP repo cannot run correctly from musl builds, do not force this pattern. Restore explicit GNU packaging instead.

### 6. Build Docker and Linux release artifacts from the same Alpine path

The Dockerfile contains a dedicated `artifact` stage that copies only the built Linux binary out of the Alpine builder image. The release workflow uses that stage to export Linux release binaries, and the Docker publish workflow uses the same build path to publish the runtime image.

That keeps Linux packaging and Linux container delivery aligned.

## Files That Define the Release System

| File | Responsibility |
| --- | --- |
| `.github/workflows/release.yml` | builds release archives, builds npm tarballs, runs smoke tests, creates GitHub release, publishes npm |
| `.github/workflows/publish-docker.yml` | builds and publishes multi-arch GHCR images |
| `.github/workflows/init-npm-placeholder.yml` | creates the initial npm package so Trusted Publishing can be configured |
| `Dockerfile` | shared Alpine build path for Linux binary export and runtime image |
| `scripts/package_release.py` | packages release archives and per-file checksums |
| `scripts/build_npm_packages.py` | stages platform npm tarballs and the meta tarball |
| `scripts/write_checksums.py` | writes `.sha256` files and the aggregate `sha256.sum` |
| `npm/meta/package.json` | base metadata for the npm meta package |
| `npm/meta/bin/sqlite-mcp-rs.js` | runtime launcher that resolves the installed platform payload |

## Release Workflow

Source: `.github/workflows/release.yml`

### Job order

1. `version`
2. `validate`
3. `build-native`
4. `build-linux`
5. `package-npm`
6. `smoke-npm-native`
7. `smoke-npm-alpine`
8. `publish`

### `version`

Purpose:

- checkout the repository
- read `Cargo.toml`
- export `version` and `tag`

This job must remain small and deterministic because every publish step depends on it.

### `validate`

Purpose:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

This is the gate that prevents packaging broken code.

### `build-native`

Purpose:

- build macOS and Windows release binaries with `cargo build --release --features vector --target ...`
- run `--help` smoke tests where the runner can execute the binary
- package each binary with `scripts/package_release.py`
- upload packaged archives as workflow artifacts

Current matrix:

- `aarch64-apple-darwin` on `macos-14`
- `x86_64-apple-darwin` on `macos-15-intel`
- `x86_64-pc-windows-msvc` on `windows-2022`
- `aarch64-pc-windows-msvc` on `windows-2022`

Current limitation from source:

- Windows arm64 is cross-built but not locally smoke-tested in the workflow.

### `build-linux`

Purpose:

- build Linux release binaries from `Dockerfile` `artifact`
- use `docker/build-push-action` with `target: artifact`
- export the produced binary to `artifact-out`
- run the binary in Alpine with `--help`
- package archives with `scripts/package_release.py`

Current matrix:

- `linux/amd64` -> `x86_64-unknown-linux-musl`
- `linux/arm64` -> `aarch64-unknown-linux-musl`

This job is the canonical Linux release path. Do not add a separate ad hoc Linux build script outside Docker unless there is a specific technical reason.

### `package-npm`

Purpose:

- download all packaged release archives
- build npm tarballs from those archives with `scripts/build_npm_packages.py`
- upload the resulting `npm-dist` directory as a workflow artifact

Important design detail:

- npm packaging consumes the release archives, not `target/` directly

That keeps npm and GitHub release payloads consistent.

### `smoke-npm-native`

Purpose:

- install the meta tarball with `npm install`
- unpack the matching platform tarball into `node_modules/<alias-name>`
- run `npx --no-install sqlite-mcp-rs --help`

This validates the launcher logic in `npm/meta/bin/sqlite-mcp-rs.js`.

Current matrix:

- macOS arm64
- macOS x64
- Windows x64
- Linux x64
- Linux arm64

Current limitation from source:

- Windows arm64 is not part of this smoke matrix.

### `smoke-npm-alpine`

Purpose:

- validate the Linux npm path in `node:24-alpine`
- install the meta tarball
- unpack the matching Linux platform tarball into `node_modules/<alias-name>`
- run `npx --no-install sqlite-mcp-rs --help`

This is the check that protects the original Alpine failure mode.

Critical implementation detail:

- the container must receive `META_TARBALL`, `PLATFORM_TARBALL`, and `ALIAS_NAME`

If `ALIAS_NAME` is missing, the tarball unpacks into the wrong directory and the launcher cannot resolve the platform package.

### `publish`

Purpose:

- download all release archives and npm tarballs
- write unified release checksums with `scripts/write_checksums.py`
- verify all expected release files exist
- write release notes
- create the GitHub release with `gh release create`
- publish npm tarballs

Publish order is intentional:

1. publish all platform tarballs first
2. publish the meta tarball last

The meta package should not be visible before its platform variants exist, otherwise npm installs can resolve an incomplete release.

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

Source: `npm/meta/bin/sqlite-mcp-rs.js`

The launcher:

1. maps `process.platform` and `process.arch` to a Rust target triple
2. maps that target triple to an alias package name
3. resolves `<alias>/package.json`
4. runs the vendored binary from `vendor/<target-triple>/`

Supported runtime mappings:

| Runtime | Target triple | Alias package |
| --- | --- | --- |
| Linux x64 | `x86_64-unknown-linux-musl` | `@bradsjm/sqlite-mcp-rs-linux-x64` |
| Linux arm64 | `aarch64-unknown-linux-musl` | `@bradsjm/sqlite-mcp-rs-linux-arm64` |
| macOS x64 | `x86_64-apple-darwin` | `@bradsjm/sqlite-mcp-rs-darwin-x64` |
| macOS arm64 | `aarch64-apple-darwin` | `@bradsjm/sqlite-mcp-rs-darwin-arm64` |
| Windows x64 | `x86_64-pc-windows-msvc` | `@bradsjm/sqlite-mcp-rs-win32-x64` |
| Windows arm64 | `aarch64-pc-windows-msvc` | `@bradsjm/sqlite-mcp-rs-win32-arm64` |

The launcher fails fast for unsupported platform and CPU combinations.

## Lessons Learned

These points are why the current design looks the way it does.

### Avoid generated installer metadata

Generated installer metadata was the source of the earlier Alpine and Linux ABI issues. Checked-in packaging code is easier to inspect, patch, and reuse.

### Do not maintain multiple version sources

The release version must come from `Cargo.toml`. Npm and Docker should consume that version, not invent their own.

### Publish the meta npm package last

Publishing the meta package first creates a broken installation window. The workflow avoids that on purpose.

### Keep Linux npm support simple

Using musl-only Linux npm payloads removed the need to manage GNU vs musl runtime selection inside the launcher.

### Keep the Linux build path shared

Using one Alpine-based Docker build path for both Linux release archives and Docker images prevents drift between “the binary users download” and “the binary the container runs”.

### Smoke test the actual install path

The npm smoke jobs install the packed tarballs and execute the launcher. That is better than only checking that tarballs exist.

## Local Validation

The repository can be partially validated locally with `act` and Docker or Podman.

Useful local checks:

- `act -n workflow_dispatch -W .github/workflows/release.yml`
- `act -n workflow_dispatch -W .github/workflows/publish-docker.yml`
- direct execution of `scripts/package_release.py`
- direct execution of `scripts/build_npm_packages.py`
- local tarball smoke tests with `npm install` and `npx --no-install`

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
7. keep Linux artifact export and Docker image build on the same Dockerfile path

Do not copy old cargo-dist based installer generation into the new repository. That reintroduces the class of problems this design removed.
