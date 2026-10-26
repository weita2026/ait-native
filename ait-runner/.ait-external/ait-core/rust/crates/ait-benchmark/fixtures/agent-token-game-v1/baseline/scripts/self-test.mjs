import assert from "node:assert/strict";
import { GameModel } from "../src/game.js";

function scriptedModel() {
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

const first = scriptedModel();
const second = scriptedModel();
assert.deepEqual(first.observableState(), second.observableState(), "fixed seed must replay exactly");

const cooldown = new GameModel({ seed: 9, bossAfterMs: 10_000 });
cooldown.step(16, { fire: true });
cooldown.step(80, { fire: true });
assert.equal(cooldown.playerBullets.length, 1, "cooldown must block rapid fire");
cooldown.step(110, { fire: true });
assert.equal(cooldown.playerBullets.length, 2, "cooldown must permit a later shot");

const paused = new GameModel({ seed: 4, bossAfterMs: 10_000 });
paused.step(100, {});
paused.step(0, { pause: true });
const pausedAt = paused.timeMs;
paused.step(100, { right: true, fire: true });
assert.equal(paused.timeMs, pausedAt, "paused simulation time must remain fixed");
paused.step(0, { pause: true });
paused.step(100, { right: true, fire: true });
const replayed = GameModel.replay(paused.exportReplay());
assert.deepEqual(replayed.observableState(), paused.observableState(), "recorded replay must converge");

const boss = new GameModel({ seed: 3, bossAfterMs: 100 });
boss.step(100, {});
assert.equal(boss.boss?.hp, 5, "boss must start with five lives");
for (let index = 0; index < 5; index += 1) {
  boss.hitBoss();
}
assert.equal(boss.phase, "victory", "five boss hits must reach victory");

const gameOver = new GameModel({ seed: 1 });
gameOver.loseLife("test");
gameOver.loseLife("test");
gameOver.loseLife("test");
assert.equal(gameOver.phase, "game_over", "three lost lives must end the game");

process.stdout.write(`${JSON.stringify({ contract: "ait-plane-shooter-self-test/v1", status: "pass" })}\n`);
