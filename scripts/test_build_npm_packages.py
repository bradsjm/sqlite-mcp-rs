#!/usr/bin/env python3

from __future__ import annotations

import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
GENERATOR = REPO_ROOT / "scripts" / "build_npm_packages.py"
VERSION = "1.2.3"
PACKAGE_NAME = "@bradsjm/sqlite-mcp-rs"

PLATFORMS = {
    "darwin-x64": {
        "alias": "@bradsjm/sqlite-mcp-rs-darwin-x64",
        "target": "x86_64-apple-darwin",
        "artifact": "sqlite-mcp-rs-x86_64-apple-darwin.tar.xz",
        "os": "darwin",
        "cpu": "x64",
        "libc": None,
    },
    "darwin-arm64": {
        "alias": "@bradsjm/sqlite-mcp-rs-darwin-arm64",
        "target": "aarch64-apple-darwin",
        "artifact": "sqlite-mcp-rs-aarch64-apple-darwin.tar.xz",
        "os": "darwin",
        "cpu": "arm64",
        "libc": None,
    },
    "win32-x64": {
        "alias": "@bradsjm/sqlite-mcp-rs-win32-x64",
        "target": "x86_64-pc-windows-msvc",
        "artifact": "sqlite-mcp-rs-x86_64-pc-windows-msvc.zip",
        "os": "win32",
        "cpu": "x64",
        "libc": None,
    },
    "linux-x64-gnu": {
        "alias": "@bradsjm/sqlite-mcp-rs-linux-x64-gnu",
        "target": "x86_64-unknown-linux-gnu",
        "artifact": "sqlite-mcp-rs-x86_64-unknown-linux-gnu.tar.xz",
        "os": "linux",
        "cpu": "x64",
        "libc": "glibc",
    },
    "linux-arm64-gnu": {
        "alias": "@bradsjm/sqlite-mcp-rs-linux-arm64-gnu",
        "target": "aarch64-unknown-linux-gnu",
        "artifact": "sqlite-mcp-rs-aarch64-unknown-linux-gnu.tar.xz",
        "os": "linux",
        "cpu": "arm64",
        "libc": "glibc",
    },
    "linux-x64-musl": {
        "alias": "@bradsjm/sqlite-mcp-rs-linux-x64-musl",
        "target": "x86_64-unknown-linux-musl",
        "artifact": "sqlite-mcp-rs-x86_64-unknown-linux-musl.tar.xz",
        "os": "linux",
        "cpu": "x64",
        "libc": "musl",
    },
    "linux-arm64-musl": {
        "alias": "@bradsjm/sqlite-mcp-rs-linux-arm64-musl",
        "target": "aarch64-unknown-linux-musl",
        "artifact": "sqlite-mcp-rs-aarch64-unknown-linux-musl.tar.xz",
        "os": "linux",
        "cpu": "arm64",
        "libc": "musl",
    },
}


def create_release_archives(artifacts_dir: Path) -> None:
    payload = b"minimal test executable\n"
    for platform in PLATFORMS.values():
        binary = "sqlite-mcp-rs.exe" if platform["os"] == "win32" else "sqlite-mcp-rs"
        member_name = f"sqlite-mcp-rs-{platform['target']}/{binary}"
        archive_path = artifacts_dir / platform["artifact"]
        if archive_path.suffix == ".zip":
            with zipfile.ZipFile(archive_path, "w") as archive:
                archive.writestr(member_name, payload)
        else:
            info = tarfile.TarInfo(member_name)
            info.size = len(payload)
            info.mode = 0o755
            with tarfile.open(archive_path, "w:xz") as archive:
                archive.addfile(info, io.BytesIO(payload))


def read_package_json(tarball: Path) -> dict:
    with tarfile.open(tarball, "r:gz") as archive:
        package_json = archive.extractfile("package/package.json")
        if package_json is None:
            raise AssertionError(f"{tarball} has no package/package.json")
        return json.load(package_json)


def make_alias_fixture(source: Path, destination: Path, alias: str) -> None:
    """Repack an npm tarball, changing only its package name."""
    with tarfile.open(source, "r:gz") as original, tarfile.open(destination, "w:gz") as fixture:
        for member in original.getmembers():
            extracted = original.extractfile(member) if member.isfile() else None
            if member.name == "package/package.json":
                if extracted is None:
                    raise AssertionError(f"{source} has an invalid package.json entry")
                package_json = json.load(extracted)
                package_json["name"] = alias
                data = (json.dumps(package_json, indent=2) + "\n").encode()
                member.size = len(data)
                fixture.addfile(member, io.BytesIO(data))
            else:
                fixture.addfile(member, extracted)


class BuildNpmPackagesTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls._temporary_directory = tempfile.TemporaryDirectory()
        cls.root = Path(cls._temporary_directory.name)
        cls.artifacts = cls.root / "artifacts"
        cls.output = cls.root / "output"
        cls.fixtures = cls.root / "fixtures"
        cls.artifacts.mkdir()
        cls.fixtures.mkdir()
        create_release_archives(cls.artifacts)
        subprocess.run(
            [
                sys.executable,
                str(GENERATOR),
                "--version",
                VERSION,
                "--artifacts-dir",
                str(cls.artifacts),
                "--output-dir",
                str(cls.output),
            ],
            cwd=REPO_ROOT,
            check=True,
        )
        cls.generated_manifest = json.loads((cls.output / "manifest.json").read_text())

    @classmethod
    def tearDownClass(cls) -> None:
        cls._temporary_directory.cleanup()

    def test_generated_package_manifests(self) -> None:
        self.assertEqual(
            set(self.generated_manifest), {"platform_packages", "meta_package"}
        )
        platform_entries = self.generated_manifest["platform_packages"]
        self.assertEqual(len(platform_entries), 7)
        self.assertEqual(
            {entry["platform"] for entry in platform_entries}, set(PLATFORMS)
        )
        self.assertEqual(
            {entry["alias_name"] for entry in platform_entries},
            {platform["alias"] for platform in PLATFORMS.values()},
        )

        for entry in platform_entries:
            key = entry["platform"]
            expected = PLATFORMS[key]
            self.assertEqual(entry["alias_name"], expected["alias"])
            tarball = self.output / entry["path"]
            package_json = read_package_json(tarball)
            self.assertEqual(package_json["name"], PACKAGE_NAME)
            self.assertEqual(package_json["version"], f"{VERSION}-{key}")
            self.assertEqual(package_json["os"], [expected["os"]])
            self.assertEqual(package_json["cpu"], [expected["cpu"]])
            if expected["libc"] is None:
                self.assertNotIn("libc", package_json)
            else:
                self.assertEqual(package_json["libc"], [expected["libc"]])

            binary = "sqlite-mcp-rs.exe" if expected["os"] == "win32" else "sqlite-mcp-rs"
            with tarfile.open(tarball, "r:gz") as archive:
                vendor_files = {
                    member.name for member in archive.getmembers() if member.isfile() and "/vendor/" in member.name
                }
            self.assertEqual(
                vendor_files,
                {f"package/vendor/{expected['target']}/{binary}"},
            )

        meta_entry = self.generated_manifest["meta_package"]
        self.assertEqual(meta_entry["name"], PACKAGE_NAME)
        meta_package_json = read_package_json(self.output / meta_entry["path"])
        self.assertEqual(meta_package_json["name"], PACKAGE_NAME)
        self.assertEqual(meta_package_json["version"], VERSION)
        self.assertEqual(
            meta_package_json["optionalDependencies"],
            {
                platform["alias"]: f"npm:{PACKAGE_NAME}@{VERSION}-{key}"
                for key, platform in PLATFORMS.items()
            },
        )
        self.assertEqual(len(list(self.output.glob("*.tgz"))), 8)

    def test_npm_filters_linux_optional_dependencies_and_lockfile(self) -> None:
        entries = {
            entry["platform"]: entry
            for entry in self.generated_manifest["platform_packages"]
        }
        fixture_paths = {}
        for key, platform in PLATFORMS.items():
            if platform["os"] != "linux":
                continue
            destination = self.fixtures / f"{key}.tgz"
            make_alias_fixture(
                self.output / entries[key]["path"], destination, platform["alias"]
            )
            fixture_paths[key] = destination

        for cpu in ("x64", "arm64"):
            same_cpu = {
                key: platform
                for key, platform in PLATFORMS.items()
                if platform["os"] == "linux" and platform["cpu"] == cpu
            }
            for libc in ("glibc", "musl"):
                with self.subTest(cpu=cpu, libc=libc), tempfile.TemporaryDirectory(
                    dir=self.root
                ) as project_name:
                    project = Path(project_name)
                    dependencies = {
                        platform["alias"]: fixture_paths[key].as_uri()
                        for key, platform in same_cpu.items()
                    }
                    (project / "package.json").write_text(
                        json.dumps(
                            {
                                "name": "npm-libc-filter-test",
                                "version": "1.0.0",
                                "private": True,
                                "optionalDependencies": dependencies,
                            },
                            indent=2,
                        )
                        + "\n"
                    )
                    env = os.environ.copy()
                    env.update(
                        {
                            "npm_config_os": "linux",
                            "npm_config_cpu": cpu,
                            "npm_config_libc": libc,
                        }
                    )
                    expected_alias = next(
                        platform["alias"]
                        for platform in same_cpu.values()
                        if platform["libc"] == libc
                    )
                    for install_number in (1, 2):
                        subprocess.run(
                            [
                                "npm",
                                "install",
                                "--ignore-scripts",
                                "--no-audit",
                                "--no-fund",
                            ],
                            cwd=project,
                            env=env,
                            check=True,
                        )
                        installed_scope = project / "node_modules" / "@bradsjm"
                        installed = (
                            {
                                f"@bradsjm/{path.name}"
                                for path in installed_scope.iterdir()
                                if path.is_dir()
                            }
                            if installed_scope.exists()
                            else set()
                        )
                        self.assertEqual(installed, {expected_alias})
                        if install_number == 1:
                            self.assertTrue((project / "package-lock.json").is_file())
                            shutil.rmtree(project / "node_modules")


if __name__ == "__main__":
    unittest.main()
