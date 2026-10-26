import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';

const addonPath = process.env.AIT_NAPI_TEST_ADDON;
const pythonExtension = process.env.AIT_PY_TEST_EXTENSION;
const nativeCli = process.env.AIT_CLI_TEST_BINARY;
for (const [name, value] of Object.entries({ addonPath, pythonExtension, nativeCli })) {
  assert.ok(value && path.isAbsolute(value), `Set the actual compiled ${name} path`);
}
const nativeModule = { exports: {} };
process.dlopen(nativeModule, addonPath);
const binding = nativeModule.exports;

test('reconcile preserves native-size limits and CLI/PyO3 success and error boundaries', () => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ait-reconcile-limit-')));
  const uninitialized = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ait-reconcile-uninitialized-')));
  const rangeError = '--limit must be between 1 and 10000.';
  const valid = [undefined, 1, 1n, 10000, 10000n];
  const outOfRange = [0, 0n, 10001, 10001n, 4294967297, 4294967297n,
    Number.MAX_SAFE_INTEGER, 18446744073709551615n];
  const malformed = [
    { value: -1, error: /nonnegative native-size integer/ },
    { value: -4294967295, error: /nonnegative native-size integer/ },
    { value: -1n, error: /nonnegative native-size integer/ },
    { value: -4294967295n, error: /nonnegative native-size integer/ },
    { value: 1.5, error: /nonnegative native-size integer/ },
    { value: -1.5, error: /nonnegative native-size integer/ },
    { value: NaN, error: /Failed to convert js number to serde_json::Number/ },
    { value: Infinity, error: /Failed to convert js number to serde_json::Number/ },
    { value: Number.MAX_SAFE_INTEGER + 1, error: /Unsafe JavaScript integer/ },
    { value: 18446744073709551616n, error: /outside the native JSON i64\/u64 range/ },
    { value: -9223372036854775809n, error: /outside the native JSON i64\/u64 range/ },
  ];
  const cases = [...valid, ...outOfRange, ...malformed.map(row => row.value)];
  const pythonCases = cases.map(value => ({
    kind: value === undefined ? 'default' : typeof value === 'number' && !Number.isInteger(value) ? 'float' : 'int',
    value: String(value),
  }));
  function cli(value) {
    return spawnSync(nativeCli, ['workflow', 'reconcile', '--json',
      ...(value === undefined ? [] : [`--limit=${String(value)}`])], { cwd: root, encoding: 'utf8' });
  }
  function stable(payload) {
    const { captured_at, ...rest } = payload;
    return rest;
  }
  try {
    binding.taskWorkflowInit(root);
    const python = spawnSync(process.env.AIT_PY_TEST_PYTHON || 'python3', ['-c', `
import importlib.machinery, importlib.util, json, sys
loader = importlib.machinery.ExtensionFileLoader('ait_py', sys.argv[1])
module = importlib.util.module_from_spec(importlib.util.spec_from_loader('ait_py', loader))
loader.exec_module(module)
results = []
for case in json.load(sys.stdin):
    try:
        kwargs = {} if case['kind'] == 'default' else {'limit': (float if case['kind'] == 'float' else int)(case['value'])}
        results.append({'payload': module.workflow_reconcile_operation(sys.argv[2], **kwargs)})
    except Exception as error:
        results.append({'error': str(error), 'type': type(error).__name__})
print(json.dumps(results))
`, pythonExtension, root], { input: JSON.stringify(pythonCases), encoding: 'utf8' });
    assert.equal(python.status, 0, python.stderr);
    const pyResults = JSON.parse(python.stdout);
    assert.throws(() => binding.workflowReconcile(uninitialized, { limit: 1 }));
    for (const [index, value] of valid.entries()) {
      const payload = binding.workflowReconcile(root, value === undefined ? {} : { limit: value });
      assert.equal(payload.limit, value === undefined ? 100 : Number(value));
      assert.equal(payload.status, 'completed');
      assert.equal(payload.mutated, false);
      const native = cli(value);
      assert.equal(native.status, 0, native.stderr);
      assert.deepEqual(stable(payload), stable(JSON.parse(native.stdout)));
      assert.deepEqual(stable(payload), stable(pyResults[index].payload));
    }
    for (const [index, value] of outOfRange.entries()) {
      assert.throws(() => binding.workflowReconcile(root, { limit: value }), error => error.message === rangeError);
      const native = cli(value);
      assert.equal(native.status, 1, native.stderr);
      assert.equal(native.stderr.trim(), `Error: ${rangeError}`);
      assert.equal(pyResults[valid.length + index].error, rangeError);
      assert.equal(pyResults[valid.length + index].type, 'ValueError');
    }
    for (const [index, { value, error }] of malformed.entries()) {
      assert.throws(() => binding.workflowReconcile(root, { limit: value, apply: true, safeOnly: true }), error);
      // A missing repository would fail discovery if an invalid scalar reached the service.
      assert.throws(() => binding.workflowReconcile(uninitialized, { limit: value }), error);
      const native = cli(value);
      assert.notEqual(native.status, 0, `CLI accepted ${String(value)}`);
      assert.ok(pyResults[valid.length + outOfRange.length + index].error, `PyO3 accepted ${String(value)}`);
    }
    assert.equal(binding.workflowReconcile(root).limit, 100);
    assert.deepEqual(binding.taskWorkflowTaskList(root), []);
    assert.deepEqual(binding.taskWorkflowChangeList(root, { local: true }), []);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
    fs.rmSync(uninitialized, { recursive: true, force: true });
  }
});
