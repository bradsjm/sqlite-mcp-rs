"use strict";

const PLATFORM_BY_TARGET = Object.freeze({
  "x86_64-unknown-linux-gnu": Object.freeze({
    packageName: "@bradsjm/sqlite-mcp-rs-linux-x64-gnu",
    os: "linux",
    cpu: "x64",
    libc: "glibc"
  }),
  "aarch64-unknown-linux-gnu": Object.freeze({
    packageName: "@bradsjm/sqlite-mcp-rs-linux-arm64-gnu",
    os: "linux",
    cpu: "arm64",
    libc: "glibc"
  }),
  "x86_64-unknown-linux-musl": Object.freeze({
    packageName: "@bradsjm/sqlite-mcp-rs-linux-x64-musl",
    os: "linux",
    cpu: "x64",
    libc: "musl"
  }),
  "aarch64-unknown-linux-musl": Object.freeze({
    packageName: "@bradsjm/sqlite-mcp-rs-linux-arm64-musl",
    os: "linux",
    cpu: "arm64",
    libc: "musl"
  }),
  "x86_64-apple-darwin": Object.freeze({
    packageName: "@bradsjm/sqlite-mcp-rs-darwin-x64",
    os: "darwin",
    cpu: "x64"
  }),
  "aarch64-apple-darwin": Object.freeze({
    packageName: "@bradsjm/sqlite-mcp-rs-darwin-arm64",
    os: "darwin",
    cpu: "arm64"
  }),
  "x86_64-pc-windows-msvc": Object.freeze({
    packageName: "@bradsjm/sqlite-mcp-rs-win32-x64",
    os: "win32",
    cpu: "x64"
  })
});

function libcError(code, message, cause) {
  const error = new Error(`${code}: ${message}`);
  error.code = code;
  if (cause !== undefined) {
    error.cause = cause;
  }
  return error;
}

function detectLinuxLibc(reportApi = process.report) {
  if (!reportApi || typeof reportApi.getReport !== "function") {
    throw libcError(
      "ERR_LIBC_REPORT_UNAVAILABLE",
      "Unable to determine Linux libc: Node.js diagnostic reports are unavailable."
    );
  }

  let report;
  try {
    report = reportApi.getReport();
  } catch (cause) {
    throw libcError(
      "ERR_LIBC_REPORT_FAILED",
      "Unable to determine Linux libc: diagnostic report generation failed.",
      cause
    );
  }

  const glibcVersion = report?.header?.glibcVersionRuntime;
  const hasGlibc = typeof glibcVersion === "string" && glibcVersion.length > 0;
  const hasMusl = Array.isArray(report?.sharedObjects) && report.sharedObjects.some(
    (sharedObject) => typeof sharedObject === "string" &&
      (sharedObject.includes("ld-musl-") || sharedObject.includes("libc.musl-"))
  );

  if (hasGlibc && hasMusl) {
    throw libcError(
      "ERR_LIBC_AMBIGUOUS",
      "Unable to determine Linux libc: diagnostic report contains conflicting glibc and musl markers."
    );
  }
  if (hasGlibc) {
    return "glibc";
  }
  if (hasMusl) {
    return "musl";
  }

  throw libcError(
    "ERR_LIBC_UNKNOWN",
    "Unable to determine Linux libc: diagnostic report contains neither a glibc runtime version nor a musl loader."
  );
}

function resolvePlatform(platform = process.platform, arch = process.arch, reportApi = process.report) {
  let targetTriple;

  if (platform === "linux" && (arch === "x64" || arch === "arm64")) {
    const architecture = arch === "x64" ? "x86_64" : "aarch64";
    const suffix = detectLinuxLibc(reportApi) === "glibc" ? "gnu" : "musl";
    targetTriple = `${architecture}-unknown-linux-${suffix}`;
  } else if (platform === "darwin" && (arch === "x64" || arch === "arm64")) {
    targetTriple = arch === "x64" ? "x86_64-apple-darwin" : "aarch64-apple-darwin";
  } else if (platform === "win32" && arch === "x64") {
    targetTriple = "x86_64-pc-windows-msvc";
  } else {
    throw new Error(`Unsupported platform: ${platform} (${arch})`);
  }

  return { targetTriple, ...PLATFORM_BY_TARGET[targetTriple] };
}

function validatePlatformPackage(packageJson, resolvedPlatform) {
  const osMatches = Array.isArray(packageJson?.os) && packageJson.os.includes(resolvedPlatform.os);
  const cpuMatches = Array.isArray(packageJson?.cpu) && packageJson.cpu.includes(resolvedPlatform.cpu);
  const libcMatches = resolvedPlatform.libc === undefined ||
    (Array.isArray(packageJson?.libc) && packageJson.libc.includes(resolvedPlatform.libc));

  if (!osMatches || !cpuMatches || !libcMatches) {
    const error = new Error(
      `Installed payload ${resolvedPlatform.packageName} does not match ${resolvedPlatform.targetTriple}: ` +
      `expected os=${resolvedPlatform.os}, cpu=${resolvedPlatform.cpu}, libc=${resolvedPlatform.libc ?? "n/a"}.`
    );
    error.code = "ERR_PLATFORM_PAYLOAD_MISMATCH";
    throw error;
  }
}

module.exports = { detectLinuxLibc, resolvePlatform, validatePlatformPackage };
