#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { runnerCoreDeclaration } from './release_runner_dependency.mjs';
const [root, version] = process.argv.slice(2);
for (const [name, table] of [['ait-external.toml', 'external'], ['ait-external.lock', 'node']]) {
  const text = readFileSync(path.join(root, name), 'utf8');
  const rows = text.split(new RegExp(`\\[\\[${table}\\]\\]`)).slice(1);
  if (rows.length === 0) throw new Error(`${name}: no component metadata`);
  for (const row of rows) {
    const repo = /^name = "([^"]+)"$/m.exec(row)?.[1];
    const actual = /^version = "([^"]+)"$/m.exec(row)?.[1];
    const runnerDependency = table === 'node' && /^parent_path = "\.ait-external\/ait-runner"$/m.test(row);
    const expected = runnerDependency ? runnerCoreDeclaration(path.join(root, '.ait-external/ait-runner')).version :
      repo === 'ait-python' ? version.replace('-rc.', 'rc') : version;
    if (actual !== expected) throw new Error(`${name}: ${repo} metadata version ${actual} differs from family ${expected}`);
  }
}
