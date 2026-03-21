#!/usr/bin/env python3

from __future__ import annotations

import argparse
import hashlib
import tarfile
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile


def sha256_digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_checksum_file(artifact_path: Path) -> None:
    checksum_path = artifact_path.with_name(f"{artifact_path.name}.sha256")
    checksum_path.write_text(f"{sha256_digest(artifact_path)}  {artifact_path.name}\n")


def add_tar_entry(archive: tarfile.TarFile, source: Path, bundle_root: str) -> None:
    archive.add(source, arcname=f"{bundle_root}/{source.name}")


def add_zip_entry(archive: ZipFile, source: Path, bundle_root: str) -> None:
    archive.write(source, arcname=f"{bundle_root}/{source.name}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Package a release archive and checksum.")
    parser.add_argument("--target-triple", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--app-name", default="sqlite-mcp-rs")
    parser.add_argument("--readme", default="README.md")
    parser.add_argument("--license", default="LICENSE")
    args = parser.parse_args()

    binary_path = Path(args.binary).resolve()
    output_dir = Path(args.output_dir).resolve()
    readme_path = Path(args.readme).resolve()
    license_path = Path(args.license).resolve()

    output_dir.mkdir(parents=True, exist_ok=True)

    if not binary_path.exists():
        raise FileNotFoundError(f"binary not found: {binary_path}")

    bundle_root = f"{args.app_name}-{args.target_triple}"
    extension = ".zip" if args.target_triple.endswith("windows-msvc") else ".tar.xz"
    archive_path = output_dir / f"{args.app_name}-{args.target_triple}{extension}"

    if extension == ".zip":
        with ZipFile(archive_path, "w", compression=ZIP_DEFLATED) as archive:
            add_zip_entry(archive, license_path, bundle_root)
            add_zip_entry(archive, readme_path, bundle_root)
            add_zip_entry(archive, binary_path, bundle_root)
    else:
        with tarfile.open(archive_path, "w:xz") as archive:
            add_tar_entry(archive, license_path, bundle_root)
            add_tar_entry(archive, readme_path, bundle_root)
            add_tar_entry(archive, binary_path, bundle_root)

    write_checksum_file(archive_path)
    print(archive_path)


if __name__ == "__main__":
    main()
