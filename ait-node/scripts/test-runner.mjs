import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REQUIRED_SUITES = Object.freeze([
  "test/runtime.test.js",
  "test/agent.test.js",
  "test/core.test.js",
  "test/api-boundary.test.js",
  "test/package-inventory.test.js",
  "test/terminal.test.js",
  "test/test-runner.test.js",
  "test/package.test.js",
  "test/payload-package.test.js",
  "test/release-adapter.test.js",
]);

function count(output, name) {
  const matches = [...output.matchAll(new RegExp(`^# ${name} (\\d+)\\s*$`, "gm"))];
  return matches.length === 0 ? null : Number(matches.at(-1)[1]);
}

export function runRequiredSuites(files, {
  cwd = ROOT,
  arguments: testArguments = [],
  writeOut = (text) => process.stdout.write(text),
  writeError = (text) => process.stderr.write(text),
} = {}) {
  if (!Array.isArray(files) || files.length === 0) {
    writeError("required test suite list is empty\n");
    return 1;
  }
  let status = 0;
  const environment = { ...process.env };
  delete environment.NODE_TEST_CONTEXT;
  for (const file of files) {
    const result = spawnSync(process.execPath, [
      "--test", "--test-reporter=tap", ...testArguments, file,
    ], { cwd, env: environment, encoding: "utf8", windowsHide: true });
    writeOut(result.stdout ?? "");
    writeError(result.stderr ?? "");
    if (result.error) {
      writeError(`${result.error.message}\n`);
      status = 1;
      continue;
    }
    const tests = count(result.stdout, "tests");
    const skipped = count(result.stdout, "skipped");
    const todo = count(result.stdout, "todo");
    const fileNames = new Set([
      file, path.resolve(cwd, file), path.basename(file),
      pathToFileURL(path.resolve(cwd, file)).href,
    ]);
    // Node may count an empty module as a successful file test. Require an
    // explicitly registered case/suite in addition to non-skipped test counts.
    const declaredCase = [...result.stdout.matchAll(/^# Subtest: (.+)$/gm)]
      .some((match) => !fileNames.has(match[1].trim()));
    if (tests === null || skipped === null || todo === null || tests - skipped - todo <= 0 || !declaredCase) {
      writeError(`required suite executed zero tests: ${file}\n`);
      status = 1;
    }
    if (result.status !== 0) status = 1;
  }
  return status;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  process.exitCode = runRequiredSuites(REQUIRED_SUITES, { arguments: process.argv.slice(2) });
}
