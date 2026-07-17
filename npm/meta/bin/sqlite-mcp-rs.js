#!/usr/bin/env node

const { spawn } = require("node:child_process");
const { createRequire } = require("node:module");
const path = require("node:path");

const {
  resolvePlatform,
  validatePlatformPackage
} = require("../lib/platform");

const requireFromMeta = createRequire(__filename);

function resolveBinaryPath() {
  const resolvedPlatform = resolvePlatform();
  let packageJsonPath;

  try {
    packageJsonPath = requireFromMeta.resolve(`${resolvedPlatform.packageName}/package.json`);
  } catch (cause) {
    if (cause?.code !== "MODULE_NOT_FOUND") {
      throw cause;
    }

    const error = new Error(
      `No native sqlite-mcp-rs payload is installed for ${resolvedPlatform.targetTriple}. ` +
      `Expected optional dependency ${resolvedPlatform.packageName}. Node ${process.version}; ` +
      `npm user agent ${process.env.npm_config_user_agent ?? "unavailable"}. ` +
      "Reinstall @bradsjm/sqlite-mcp-rs without --omit=optional or --no-optional."
    );
    error.code = "ERR_PLATFORM_PAYLOAD_MISSING";
    error.cause = cause;
    throw error;
  }

  const packageJson = requireFromMeta(packageJsonPath);
  validatePlatformPackage(packageJson, resolvedPlatform);

  const packageRoot = path.dirname(packageJsonPath);
  const binaryName = resolvedPlatform.os === "win32" ? "sqlite-mcp-rs.exe" : "sqlite-mcp-rs";
  return path.join(packageRoot, "vendor", resolvedPlatform.targetTriple, binaryName);
}

function main() {
  let child;
  try {
    child = spawn(resolveBinaryPath(), process.argv.slice(2), {
      stdio: "inherit"
    });
  } catch (error) {
    console.error(error);
    process.exit(1);
    return;
  }

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
}

main();
