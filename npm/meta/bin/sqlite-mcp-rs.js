#!/usr/bin/env node

const { spawn } = require("node:child_process");
const { createRequire } = require("node:module");
const path = require("node:path");

const requireFromMeta = createRequire(__filename);

const PLATFORM_PACKAGE_BY_TARGET = {
  "x86_64-unknown-linux-musl": "@bradsjm/sqlite-mcp-rs-linux-x64",
  "aarch64-unknown-linux-musl": "@bradsjm/sqlite-mcp-rs-linux-arm64",
  "x86_64-apple-darwin": "@bradsjm/sqlite-mcp-rs-darwin-x64",
  "aarch64-apple-darwin": "@bradsjm/sqlite-mcp-rs-darwin-arm64",
  "x86_64-pc-windows-msvc": "@bradsjm/sqlite-mcp-rs-win32-x64",
  "aarch64-pc-windows-msvc": "@bradsjm/sqlite-mcp-rs-win32-arm64"
};

function resolveTargetTriple() {
  switch (process.platform) {
    case "linux":
      switch (process.arch) {
        case "x64":
          return "x86_64-unknown-linux-musl";
        case "arm64":
          return "aarch64-unknown-linux-musl";
        default:
          break;
      }
      break;
    case "darwin":
      switch (process.arch) {
        case "x64":
          return "x86_64-apple-darwin";
        case "arm64":
          return "aarch64-apple-darwin";
        default:
          break;
      }
      break;
    case "win32":
      switch (process.arch) {
        case "x64":
          return "x86_64-pc-windows-msvc";
        case "arm64":
          return "aarch64-pc-windows-msvc";
        default:
          break;
      }
      break;
    default:
      break;
  }

  throw new Error(`Unsupported platform: ${process.platform} (${process.arch})`);
}

function resolveBinaryPath() {
  const targetTriple = resolveTargetTriple();
  const platformPackage = PLATFORM_PACKAGE_BY_TARGET[targetTriple];
  if (!platformPackage) {
    throw new Error(`Unsupported target triple: ${targetTriple}`);
  }

  const packageJsonPath = requireFromMeta.resolve(`${platformPackage}/package.json`);
  const packageRoot = path.dirname(packageJsonPath);
  const binaryName = process.platform === "win32" ? "sqlite-mcp-rs.exe" : "sqlite-mcp-rs";
  return path.join(packageRoot, "vendor", targetTriple, binaryName);
}

const child = spawn(resolveBinaryPath(), process.argv.slice(2), {
  stdio: "inherit"
});

child.on("error", (error) => {
  console.error(error);
  process.exit(1);
});

["SIGINT", "SIGTERM", "SIGHUP"].forEach((signal) => {
  process.on(signal, () => {
    if (!child.killed) {
      child.kill(signal);
    }
  });
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }

  process.exit(code ?? 1);
});
