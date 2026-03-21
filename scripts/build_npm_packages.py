#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
NPM_META_ROOT = REPO_ROOT / "npm" / "meta"

PLATFORMS = {
    "darwin-x64": {
        "alias_name": "@bradsjm/sqlite-mcp-rs-darwin-x64",
        "target_triple": "x86_64-apple-darwin",
        "artifact_name": "sqlite-mcp-rs-x86_64-apple-darwin.tar.xz",
        "os": "darwin",
        "cpu": "x64",
    },
    "darwin-arm64": {
        "alias_name": "@bradsjm/sqlite-mcp-rs-darwin-arm64",
        "target_triple": "aarch64-apple-darwin",
        "artifact_name": "sqlite-mcp-rs-aarch64-apple-darwin.tar.xz",
        "os": "darwin",
        "cpu": "arm64",
    },
    "win32-x64": {
        "alias_name": "@bradsjm/sqlite-mcp-rs-win32-x64",
        "target_triple": "x86_64-pc-windows-msvc",
        "artifact_name": "sqlite-mcp-rs-x86_64-pc-windows-msvc.zip",
        "os": "win32",
        "cpu": "x64",
    },
    "linux-x64": {
        "alias_name": "@bradsjm/sqlite-mcp-rs-linux-x64",
        "target_triple": "x86_64-unknown-linux-musl",
        "artifact_name": "sqlite-mcp-rs-x86_64-unknown-linux-musl.tar.xz",
        "os": "linux",
        "cpu": "x64",
    },
    "linux-arm64": {
        "alias_name": "@bradsjm/sqlite-mcp-rs-linux-arm64",
        "target_triple": "aarch64-unknown-linux-musl",
        "artifact_name": "sqlite-mcp-rs-aarch64-unknown-linux-musl.tar.xz",
        "os": "linux",
        "cpu": "arm64",
    },
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build npm meta and platform packages.")
    parser.add_argument("--version", required=True)
    parser.add_argument("--artifacts-dir", required=True)
    parser.add_argument("--output-dir", required=True)
    return parser.parse_args()


def pack_npm_package(staging_dir: Path, output_dir: Path) -> Path:
    stdout = subprocess.check_output(
        ["npm", "pack", "--json", "--pack-destination", str(output_dir)],
        cwd=staging_dir,
        text=True,
    )
    pack_output = json.loads(stdout)
    return output_dir / pack_output[0]["filename"]


def copy_release_binary(archive_path: Path, target_triple: str, staging_dir: Path) -> None:
    binary_name = "sqlite-mcp-rs.exe" if "windows" in target_triple else "sqlite-mcp-rs"
    vendor_dir = staging_dir / "vendor" / target_triple
    vendor_dir.mkdir(parents=True, exist_ok=True)
    output_path = vendor_dir / binary_name

    if archive_path.suffix == ".zip":
        with zipfile.ZipFile(archive_path) as archive:
            member = next(name for name in archive.namelist() if name.endswith(f"/{binary_name}"))
            with archive.open(member) as source, output_path.open("wb") as dest:
                shutil.copyfileobj(source, dest)
    else:
        with tarfile.open(archive_path, "r:xz") as archive:
            member = next(entry for entry in archive.getmembers() if entry.name.endswith(f"/{binary_name}"))
            extracted = archive.extractfile(member)
            if extracted is None:
                raise RuntimeError(f"failed to extract {binary_name} from {archive_path}")
            with extracted, output_path.open("wb") as dest:
                shutil.copyfileobj(extracted, dest)

    if "windows" not in target_triple:
        output_path.chmod(0o755)


def write_json(path: Path, payload: dict) -> None:
    path.write_text(json.dumps(payload, indent=2) + "\n")


def stage_platform_package(
    version: str, artifacts_dir: Path, output_dir: Path, platform_key: str, platform: dict
) -> Path:
    staging_dir = Path(tempfile.mkdtemp(prefix=f"sqlite-mcp-rs-{platform_key}-"))
    shutil.copy2(REPO_ROOT / "README.md", staging_dir / "README.md")
    shutil.copy2(REPO_ROOT / "LICENSE", staging_dir / "LICENSE")

    package_json = {
        "name": "@bradsjm/sqlite-mcp-rs",
        "version": f"{version}-{platform_key}",
        "license": "MIT",
        "files": ["vendor"],
        "os": [platform["os"]],
        "cpu": [platform["cpu"]],
        "repository": {
            "type": "git",
            "url": "git+https://github.com/bradsjm/sqlite-mcp-rs.git",
        },
        "homepage": "https://github.com/bradsjm/sqlite-mcp-rs#readme",
        "bugs": {"url": "https://github.com/bradsjm/sqlite-mcp-rs/issues"},
    }
    write_json(staging_dir / "package.json", package_json)
    copy_release_binary(artifacts_dir / platform["artifact_name"], platform["target_triple"], staging_dir)
    return pack_npm_package(staging_dir, output_dir)


def stage_meta_package(version: str, output_dir: Path) -> Path:
    staging_dir = Path(tempfile.mkdtemp(prefix="sqlite-mcp-rs-meta-"))
    shutil.copytree(NPM_META_ROOT / "bin", staging_dir / "bin")
    shutil.copy2(REPO_ROOT / "README.md", staging_dir / "README.md")
    shutil.copy2(REPO_ROOT / "LICENSE", staging_dir / "LICENSE")

    package_json = json.loads((NPM_META_ROOT / "package.json").read_text())
    package_json["version"] = version
    package_json["optionalDependencies"] = {
        platform["alias_name"]: f"npm:@bradsjm/sqlite-mcp-rs@{version}-{platform_key}"
        for platform_key, platform in PLATFORMS.items()
    }
    write_json(staging_dir / "package.json", package_json)
    return pack_npm_package(staging_dir, output_dir)


def main() -> None:
    args = parse_args()
    artifacts_dir = Path(args.artifacts_dir).resolve()
    output_dir = Path(args.output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)

    manifest = {"platform_packages": [], "meta_package": None}
    for platform_key, platform in PLATFORMS.items():
        tarball = stage_platform_package(args.version, artifacts_dir, output_dir, platform_key, platform)
        manifest["platform_packages"].append(
            {"alias_name": platform["alias_name"], "path": tarball.name, "platform": platform_key}
        )

    meta_tarball = stage_meta_package(args.version, output_dir)
    manifest["meta_package"] = {"name": "@bradsjm/sqlite-mcp-rs", "path": meta_tarball.name}
    write_json(output_dir / "manifest.json", manifest)


if __name__ == "__main__":
    main()
