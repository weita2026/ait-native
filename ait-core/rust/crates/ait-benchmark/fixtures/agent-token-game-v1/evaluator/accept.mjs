import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { lstat, readFile, readdir } from "node:fs/promises";
import { resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";

function parseArguments(values) {
  const parsed = new Map();
  for (let index = 0; index < values.length; index += 2) {
    const key = values[index];
    const value = values[index + 1];
    if (!key?.startsWith("--") || !value) {
      throw new Error("Usage: accept.mjs --workload GD-01 --candidate PATH --acceptance PATH");
    }
    parsed.set(key.slice(2), value);
  }
  for (const required of ["workload", "candidate", "acceptance"]) {
    if (!parsed.has(required)) {
      throw new Error(`Missing --${required}`);
    }
  }
  return parsed;
}

const argumentsMap = parseArguments(process.argv.slice(2));
const workloadId = argumentsMap.get("workload");
const candidate = resolve(argumentsMap.get("candidate"));
const acceptancePath = resolve(argumentsMap.get("acceptance"));
const acceptance = JSON.parse(await readFile(acceptancePath, "utf8"));
assert.equal(acceptance.contract, "ait-agent-token-game-acceptance/v1");
assert.equal(acceptance.workload_id, workloadId);

const results = [];
const blockers = [];
let score = 0;

async function category(name, points, check) {
  try {
    await check();
    score += points;
    results.push({ name, points, status: "pass" });
  } catch (error) {
    blockers.push(name);
    results.push({
      name,
      points: 0,
      maximumPoints: points,
      status: "fail",
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

function candidatePath(relative) {
  const target = resolve(candidate, relative);
  if (target !== candidate && !target.startsWith(`${candidate}${sep}`)) {
    throw new Error(`Candidate path escapes root: ${relative}`);
  }
  return target;
}

async function requireRegularFile(relative) {
  const path = candidatePath(relative);
  const metadata = await lstat(path);
  assert.equal(metadata.isFile(), true, `${relative} must be a regular file`);
  assert.equal(metadata.isSymbolicLink(), false, `${relative} must not be a symbolic link`);
  return path;
}

async function importCandidate(relative) {
  const path = await requireRegularFile(relative);
  return import(`${pathToFileURL(path).href}?acceptance=${Date.now()}-${Math.random()}`);
}

function scriptedState(GameModel) {
  const model = new GameModel({ seed: 1337, bossAfterMs: 10_000 });
  for (let index = 0; index < 120; index += 1) {
    model.step(50, {
      left: index >= 50 && index < 70,
      right: index < 20,
      fire: index % 5 === 0,
    });
  }
  return model;
}

async function checkWorkloadBehavior(module) {
  const { GameModel } = module;
  assert.equal(typeof GameModel, "function", "src/game.js must export GameModel");
  const initial = new GameModel({ seed: 7, bossAfterMs: 100 });
  assert.equal(initial.lives, 3);
  assert.equal(initial.score, 0);
  assert.equal(initial.phase, "playing");
  initial.step(100, {});
  assert.equal(initial.boss?.hp, 5, "boss must start with five hit points");
  for (let index = 0; index < 5; index += 1) {
    initial.hitBoss();
  }
  assert.equal(initial.phase, "victory");

  const gameOver = new GameModel({ seed: 8 });
  gameOver.loseLife("acceptance");
  gameOver.loseLife("acceptance");
  gameOver.loseLife("acceptance");
  assert.equal(gameOver.phase, "game_over");

  if (workloadId === "GD-01") {
    const model = new GameModel({ seed: 9, bossAfterMs: 10_000 });
    model.step(16, { fire: true });
    model.step(80, { fire: true });
    assert.equal(model.playerBullets.length, 1, "rapid fire must be blocked before 180 ms");
    model.step(110, { fire: true });
    assert.equal(model.playerBullets.length, 2, "a shot after 180 ms must be accepted");
  }

  if (workloadId === "GD-02") {
    const normalAppearance = module.enemyAppearance("normal");
    const armoredAppearance = module.enemyAppearance("armored");
    assert.equal(armoredAppearance.visual, "armored");
    assert.notDeepEqual(
      { color: armoredAppearance.color, accent: armoredAppearance.accent },
      { color: normalAppearance.color, accent: normalAppearance.accent },
      "armored enemy canvas colors must be visually distinct",
    );
    const model = new GameModel({ seed: 10, bossAfterMs: 10_000 });
    const armored = model.spawnEnemy("armored", 120);
    assert.equal(armored.type, "armored");
    assert.equal(armored.hp, 2);
    assert.equal(armored.scoreValue, 250);
    assert.equal(armored.visual, "armored");
    model.hitEnemy(armored.id);
    assert.equal(model.enemies.find((enemy) => enemy.id === armored.id)?.hp, 1);
    assert.equal(model.score, 0);
    model.hitEnemy(armored.id);
    assert.equal(model.enemies.some((enemy) => enemy.id === armored.id), false);
    assert.equal(model.score, 250);
  }

  if (workloadId === "GD-03") {
    const model = new GameModel({ seed: 11, bossAfterMs: 10_000 });
    model.step(100, {});
    model.step(0, { pause: true });
    const pausedAt = model.timeMs;
    const playerAt = model.player.x;
    model.step(100, { right: true, fire: true });
    assert.equal(model.timeMs, pausedAt, "paused simulation time must remain fixed");
    assert.equal(model.player.x, playerAt, "paused input must not move the player");
    assert.equal(model.playerBullets.length, 0, "paused input must not fire");
    model.step(0, { pause: true });
    model.step(100, { right: true, fire: true });
    assert.deepEqual(
      GameModel.replay(model.exportReplay()).observableState(),
      model.observableState(),
      "pause/resume replay must converge",
    );
  }

  if (workloadId === "GD-05") {
    const replayModule = await importCandidate("src/replay.js");
    assert.equal(replayModule.REPLAY_SCHEMA_VERSION, 1);
    const recording = scriptedState(GameModel).exportReplay();
    const first = replayModule.encodeReplay(recording);
    const second = replayModule.encodeReplay(recording);
    assert.equal(first, second, "replay encoding must be deterministic");
    assert.deepEqual(replayModule.decodeReplay(first), recording);
    assert.throws(() => replayModule.decodeReplay("not-json"));

    const settingsModule = await importCandidate("src/settings.js");
    assert.equal(settingsModule.SETTINGS_SCHEMA_VERSION, 1);
    assert.deepEqual(settingsModule.normalizeSettings({
      volume: 5,
      soundEnabled: 0,
      difficulty: "unknown",
    }), {
      schemaVersion: 1,
      volume: 1,
      soundEnabled: false,
      difficulty: "normal",
    });

    const mobileModule = await importCandidate("src/mobile-input.js");
    assert.deepEqual(mobileModule.normalizeTouchInput({ left: 1, fire: "yes" }), {
      left: true,
      right: false,
      fire: true,
    });
  }
}

async function checkDeterminism(module) {
  const { GameModel } = module;
  const first = scriptedState(GameModel);
  const second = scriptedState(GameModel);
  assert.deepEqual(first.observableState(), second.observableState(), "fixed seed must be exact");
  assert.deepEqual(
    GameModel.replay(first.exportReplay()).observableState(),
    first.observableState(),
    "exported replay must converge",
  );
}

async function checkStructure(module) {
  for (const path of acceptance.required_paths) {
    await requireRegularFile(path);
  }
  assert.equal(typeof module.GameModel, "function");
  if (workloadId === "GD-04") {
    await requireRegularFile("src/model.js");
    await requireRegularFile("src/renderer.js");
    await requireRegularFile("src/input.js");
    const entry = await readFile(candidatePath("src/game.js"), "utf8");
    assert.match(entry, /model\.js/);
    assert.match(entry, /renderer\.js/);
    assert.match(entry, /input\.js/);
    assert.match(entry, /__AIT_GAME__/);
  }
  if (workloadId === "GD-05") {
    const html = await readFile(candidatePath("index.html"), "utf8");
    const game = await readFile(candidatePath("src/game.js"), "utf8");
    assert.match(html, /data-touch-control/);
    assert.match(html, /data-benchmark-panel/);
    assert.match(game, /visibilitychange/);
    assert.match(game, /["']blur["']/);
  }
}

async function checkStartup() {
  const packageManifest = JSON.parse(await readFile(candidatePath("package.json"), "utf8"));
  assert.equal(packageManifest.type, "module");
  assert.match(packageManifest.scripts?.serve || "", /node/);
  assert.match(packageManifest.scripts?.test || "", /node/);
  const html = await readFile(candidatePath("index.html"), "utf8");
  assert.match(html, /<canvas[^>]+id="game"/);
  assert.match(html, /src="src\/game\.js"/);
  const selfTest = spawnSync(process.execPath, ["scripts/self-test.mjs"], {
    cwd: candidate,
    encoding: "utf8",
    timeout: 30_000,
  });
  assert.equal(
    selfTest.status,
    0,
    `project-local self-test failed: ${selfTest.stderr || selfTest.stdout}`,
  );

  if (workloadId === "GD-05") {
    const releaseCheck = spawnSync(process.execPath, ["scripts/release-check.mjs"], {
      cwd: candidate,
      encoding: "utf8",
      timeout: 30_000,
    });
    assert.equal(
      releaseCheck.status,
      0,
      `release check failed: ${releaseCheck.stderr || releaseCheck.stdout}`,
    );
    const releaseDocument = await readFile(candidatePath("RELEASE.txt"), "utf8");
    assert.match(
      releaseDocument,
      /(?:scripts\/release-check\.mjs|npm run release-check)/,
    );
  }
}

async function walk(root, visit) {
  const entries = await readdir(root, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    if ([".ait", ".git", "node_modules"].includes(entry.name)) {
      continue;
    }
    const path = resolve(root, entry.name);
    if (entry.isSymbolicLink()) {
      throw new Error(`symbolic links are not allowed: ${path}`);
    }
    if (entry.isDirectory()) {
      await walk(path, visit);
    } else if (entry.isFile()) {
      await visit(path);
    }
  }
}

async function checkHygiene() {
  let fileCount = 0;
  await walk(candidate, async (path) => {
    fileCount += 1;
    assert.equal(path.endsWith(".py"), false, `Python file is forbidden: ${path}`);
    if (/\.(?:css|html|js|json|md|mjs|txt)$/.test(path)) {
      const source = await readFile(path, "utf8");
      const external = source.match(/https?:\/\/(?!127\.0\.0\.1|localhost)[^\s"')]+/);
      assert.equal(external, null, `external URL is forbidden: ${external?.[0]}`);
      const secret = source.match(
        /(?:sk-[A-Za-z0-9_-]{20,}|AKIA[0-9A-Z]{16}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)/,
      );
      assert.equal(secret, null, `credential-shaped secret is forbidden in ${path}`);
    }
  });
  assert.ok(fileCount >= 7, "candidate file inventory is unexpectedly small");
  for (const forbidden of acceptance.forbidden_paths) {
    try {
      await lstat(candidatePath(forbidden));
      assert.fail(`forbidden path exists: ${forbidden}`);
    } catch (error) {
      if (error?.code !== "ENOENT") {
        throw error;
      }
    }
  }
  const workload = JSON.parse(await readFile(candidatePath(".benchmark/workload.json"), "utf8"));
  assert.equal(workload.workload_id, workloadId);
  assert.equal(workload.workflow_mode, "solo_local");
  assert.equal(workload.ait_server_allowed, false);
}

let gameModule;
try {
  gameModule = await importCandidate("src/game.js");
} catch (error) {
  blockers.push("fixture-fails-to-launch");
  results.push({
    name: "fixture-fails-to-launch",
    points: 0,
    maximumPoints: 50,
    status: "fail",
    error: error instanceof Error ? error.message : String(error),
  });
}

if (gameModule) {
  await category("required-gameplay-and-workload-behavior", 50, () =>
    checkWorkloadBehavior(gameModule),
  );
  await category("determinism-and-replayable-harness", 15, () =>
    checkDeterminism(gameModule),
  );
  await category("code-structure-and-regression-safety", 15, () =>
    checkStructure(gameModule),
  );
} else {
  results.push({ name: "determinism-and-replayable-harness", points: 0, maximumPoints: 15, status: "blocked" });
  results.push({ name: "code-structure-and-regression-safety", points: 0, maximumPoints: 15, status: "blocked" });
}
await category("startup-validation-and-integration", 10, checkStartup);
await category("benchmark-hygiene-and-originality", 10, checkHygiene);

const accepted = score >= acceptance.minimum_score && blockers.length === 0;
const report = {
  contract: "ait-agent-token-game-acceptance-report/v1",
  workloadId,
  candidate,
  score,
  minimumScore: acceptance.minimum_score,
  accepted,
  blockers,
  results,
};
process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
process.exitCode = accepted ? 0 : 1;
