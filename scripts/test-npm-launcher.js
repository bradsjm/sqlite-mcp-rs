#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const {
  detectLinuxLibc,
  resolvePlatform,
  validatePlatformPackage
} = require("../npm/meta/lib/platform");

function report(header = {}, sharedObjects = []) {
  return { getReport: () => ({ header, sharedObjects }) };
}

function assertError(fn, code, message, cause) {
  assert.throws(fn, (error) => {
    assert.equal(error.code, code);
    assert.equal(error.message, `${code}: ${message}`);
    if (cause !== undefined) {
      assert.equal(error.cause, cause);
    }
    return true;
  });
}

const glibcReport = report({ glibcVersionRuntime: "2.39" });
const muslReport = report({}, ["/lib/ld-musl-aarch64.so.1"]);
const muslLibcReport = report({}, ["/lib/libc.musl-x86_64.so.1"]);

assert.deepEqual(resolvePlatform("linux", "x64", glibcReport), {
  targetTriple: "x86_64-unknown-linux-gnu",
  packageName: "@bradsjm/sqlite-mcp-rs-linux-x64-gnu",
  os: "linux",
  cpu: "x64",
  libc: "glibc"
});
assert.deepEqual(resolvePlatform("linux", "arm64", glibcReport), {
  targetTriple: "aarch64-unknown-linux-gnu",
  packageName: "@bradsjm/sqlite-mcp-rs-linux-arm64-gnu",
  os: "linux",
  cpu: "arm64",
  libc: "glibc"
});
assert.deepEqual(resolvePlatform("linux", "x64", muslLibcReport), {
  targetTriple: "x86_64-unknown-linux-musl",
  packageName: "@bradsjm/sqlite-mcp-rs-linux-x64-musl",
  os: "linux",
  cpu: "x64",
  libc: "musl"
});
assert.deepEqual(resolvePlatform("linux", "arm64", muslReport), {
  targetTriple: "aarch64-unknown-linux-musl",
  packageName: "@bradsjm/sqlite-mcp-rs-linux-arm64-musl",
  os: "linux",
  cpu: "arm64",
  libc: "musl"
});

let reportQueried = false;
assert.throws(
  () => resolvePlatform("linux", "ppc64", { getReport: () => { reportQueried = true; } }),
  { message: "Unsupported platform: linux (ppc64)" }
);
assert.equal(reportQueried, false);
assert.deepEqual(resolvePlatform("darwin", "x64", null), {
  targetTriple: "x86_64-apple-darwin",
  packageName: "@bradsjm/sqlite-mcp-rs-darwin-x64",
  os: "darwin",
  cpu: "x64"
});

assertError(
  () => detectLinuxLibc(null),
  "ERR_LIBC_REPORT_UNAVAILABLE",
  "Unable to determine Linux libc: Node.js diagnostic reports are unavailable."
);
assertError(
  () => detectLinuxLibc({}),
  "ERR_LIBC_REPORT_UNAVAILABLE",
  "Unable to determine Linux libc: Node.js diagnostic reports are unavailable."
);
const reportFailure = new Error("report failed");
assertError(
  () => detectLinuxLibc({ getReport: () => { throw reportFailure; } }),
  "ERR_LIBC_REPORT_FAILED",
  "Unable to determine Linux libc: diagnostic report generation failed.",
  reportFailure
);
assertError(
  () => detectLinuxLibc(report(
    { glibcVersionRuntime: "2.39" },
    ["/lib/ld-musl-x86_64.so.1"]
  )),
  "ERR_LIBC_AMBIGUOUS",
  "Unable to determine Linux libc: diagnostic report contains conflicting glibc and musl markers."
);
assertError(
  () => detectLinuxLibc(report()),
  "ERR_LIBC_UNKNOWN",
  "Unable to determine Linux libc: diagnostic report contains neither a glibc runtime version nor a musl loader."
);

const resolvedGnu = resolvePlatform("linux", "x64", glibcReport);
validatePlatformPackage(
  { os: ["linux"], cpu: ["x64"], libc: ["glibc"] },
  resolvedGnu
);
for (const packageJson of [
  { os: ["darwin"], cpu: ["x64"], libc: ["glibc"] },
  { os: ["linux"], cpu: ["arm64"], libc: ["glibc"] },
  { os: ["linux"], cpu: ["x64"], libc: ["musl"] }
]) {
  assert.throws(
    () => validatePlatformPackage(packageJson, resolvedGnu),
    {
      code: "ERR_PLATFORM_PAYLOAD_MISMATCH",
      message: "Installed payload @bradsjm/sqlite-mcp-rs-linux-x64-gnu does not match x86_64-unknown-linux-gnu: expected os=linux, cpu=x64, libc=glibc."
    }
  );
}

console.log("npm launcher tests passed");
