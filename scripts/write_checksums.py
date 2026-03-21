#!/usr/bin/env python3

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path


def sha256_digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description="Write per-file and unified SHA256 checksums.")
    parser.add_argument("--dist-dir", required=True)
    args = parser.parse_args()

    dist_dir = Path(args.dist_dir).resolve()
    artifacts = sorted(
        path
        for path in dist_dir.iterdir()
        if path.is_file() and not path.name.endswith(".sha256") and path.name != "sha256.sum"
    )

    checksum_lines = []
    for artifact in artifacts:
        digest = sha256_digest(artifact)
        checksum_text = f"{digest}  {artifact.name}\n"
        artifact.with_name(f"{artifact.name}.sha256").write_text(checksum_text)
        checksum_lines.append(checksum_text)

    (dist_dir / "sha256.sum").write_text("".join(checksum_lines))


if __name__ == "__main__":
    main()
