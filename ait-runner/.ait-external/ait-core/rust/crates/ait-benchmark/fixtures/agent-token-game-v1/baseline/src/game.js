export const GAME_WIDTH = 480;
export const GAME_HEIGHT = 640;
export const FIRE_COOLDOWN_MS = 180;
export const PAUSED_TIME_ADVANCES = false;
export const NORMAL_BOSS_AFTER_MS = 180_000;

const PLAYER_SPEED = 220;
const PLAYER_BULLET_SPEED = 420;
const ENEMY_BULLET_SPEED = 165;
const ENEMY_SPEED = 58;
const ENEMY_SPAWN_MS = 760;

export class SeededRandom {
  constructor(seed = 1337) {
    this.state = Number(seed) >>> 0;
  }

  next() {
    this.state = (Math.imul(this.state, 1664525) + 1013904223) >>> 0;
    return this.state / 0x1_0000_0000;
  }

  between(minimum, maximum) {
    return minimum + (maximum - minimum) * this.next();
  }
}

export function enemyAppearance(type) {
  if (type !== "normal") {
    return { visual: "unknown", color: "#b8c0ed", accent: "#59618f" };
  }
  return { visual: "scout", color: "#ff5a6f", accent: "#ffb347" };
}

function overlaps(left, right) {
  return (
    left.x < right.x + right.width &&
    left.x + left.width > right.x &&
    left.y < right.y + right.height &&
    left.y + left.height > right.y
  );
}

function copyInput(input = {}) {
  return {
    left: Boolean(input.left),
    right: Boolean(input.right),
    fire: Boolean(input.fire),
    pause: Boolean(input.pause),
    restart: Boolean(input.restart),
  };
}

export class GameModel {
  constructor({ seed = 1337, bossAfterMs = NORMAL_BOSS_AFTER_MS } = {}) {
    this.seed = Number(seed) >>> 0;
    this.bossAfterMs = Math.max(1, Number(bossAfterMs) || NORMAL_BOSS_AFTER_MS);
    this.reset();
  }

  reset() {
    this.random = new SeededRandom(this.seed);
    this.timeMs = 0;
    this.paused = false;
    this.phase = "playing";
    this.lives = 3;
    this.score = 0;
    this.player = { x: 226, y: 574, width: 28, height: 34 };
    this.playerBullets = [];
    this.enemies = [];
    this.enemyBullets = [];
    this.boss = null;
    this.events = [];
    this.replayFrames = [];
    this.lastShotAtMs = Number.NEGATIVE_INFINITY;
    this.nextEnemyAtMs = 480;
    this.nextEntityOrdinal = 1;
    this.emit("game_started", { seed: this.seed, bossAfterMs: this.bossAfterMs });
    return this.observableState();
  }

  emit(type, detail = {}) {
    const event = {
      sequence: this.events.length,
      atMs: Math.round(this.timeMs),
      type,
      ...detail,
    };
    this.events.push(event);
    return event;
  }

  spawnEnemy(type = "normal", requestedX) {
    const appearance = enemyAppearance(type);
    const width = 30;
    const enemy = {
      id: `enemy-${this.nextEntityOrdinal++}`,
      type,
      x: Number.isFinite(requestedX)
        ? Math.max(0, Math.min(GAME_WIDTH - width, requestedX))
        : Math.round(this.random.between(18, GAME_WIDTH - width - 18)),
      y: -34,
      width,
      height: 28,
      hp: 1,
      scoreValue: 100,
      visual: appearance.visual,
      color: appearance.color,
      accent: appearance.accent,
      nextShotAtMs: this.timeMs + 780 + Math.round(this.random.between(0, 500)),
    };
    this.enemies.push(enemy);
    this.emit("enemy_spawned", { id: enemy.id, enemyType: enemy.type, x: enemy.x });
    return enemy;
  }

  hitEnemy(id, damage = 1) {
    const enemy = this.enemies.find((candidate) => candidate.id === id);
    if (!enemy || damage <= 0) {
      return false;
    }
    enemy.hp -= damage;
    this.emit("enemy_hit", { id: enemy.id, enemyType: enemy.type, hp: enemy.hp });
    if (enemy.hp <= 0) {
      this.score += enemy.scoreValue;
      this.enemies = this.enemies.filter((candidate) => candidate.id !== enemy.id);
      this.emit("enemy_destroyed", {
        id: enemy.id,
        enemyType: enemy.type,
        scoreValue: enemy.scoreValue,
      });
    }
    return true;
  }

  hitBoss(damage = 1) {
    if (!this.boss || this.phase !== "playing" || damage <= 0) {
      return false;
    }
    this.boss.hp -= damage;
    this.emit("boss_hit", { hp: this.boss.hp });
    if (this.boss.hp <= 0) {
      this.boss.hp = 0;
      this.phase = "victory";
      this.score += 2000;
      this.emit("victory", { score: this.score });
    }
    return true;
  }

  loseLife(reason = "collision") {
    if (this.phase !== "playing") {
      return false;
    }
    this.lives = Math.max(0, this.lives - 1);
    this.emit("life_lost", { lives: this.lives, reason });
    if (this.lives === 0) {
      this.phase = "game_over";
      this.emit("game_over", { score: this.score });
    }
    return true;
  }

  fire() {
    if (this.timeMs - this.lastShotAtMs < FIRE_COOLDOWN_MS) {
      return false;
    }
    const bullet = {
      id: `player-bullet-${this.nextEntityOrdinal++}`,
      x: this.player.x + this.player.width / 2 - 2,
      y: this.player.y - 10,
      width: 4,
      height: 12,
    };
    this.playerBullets.push(bullet);
    this.lastShotAtMs = this.timeMs;
    this.emit("player_fired", { id: bullet.id });
    return true;
  }

  step(deltaMs, rawInput = {}, { record = true } = {}) {
    const delta = Math.max(0, Math.min(100, Number(deltaMs) || 0));
    const input = copyInput(rawInput);
    if (record) {
      this.replayFrames.push({ deltaMs: delta, input });
    }
    if (input.restart) {
      return this.reset();
    }
    if (input.pause && this.phase === "playing") {
      this.paused = !this.paused;
      this.emit(this.paused ? "paused" : "resumed");
    }
    if (this.paused) {
      if (PAUSED_TIME_ADVANCES) {
        this.timeMs += delta;
      }
      return this.observableState();
    }
    if (this.phase !== "playing") {
      return this.observableState();
    }

    this.timeMs += delta;
    const direction = Number(input.right) - Number(input.left);
    this.player.x = Math.max(
      0,
      Math.min(
        GAME_WIDTH - this.player.width,
        this.player.x + direction * PLAYER_SPEED * (delta / 1000),
      ),
    );
    if (input.fire) {
      this.fire();
    }

    while (this.timeMs >= this.nextEnemyAtMs && !this.boss) {
      this.spawnEnemy();
      this.nextEnemyAtMs += ENEMY_SPAWN_MS;
    }
    if (!this.boss && this.timeMs >= this.bossAfterMs) {
      this.boss = {
        id: "command-carrier",
        x: 140,
        y: 48,
        width: 200,
        height: 72,
        hp: 5,
      };
      this.emit("boss_spawned", { hp: this.boss.hp });
    }

    this.updateProjectiles(delta);
    this.updateEnemies(delta);
    this.resolveCollisions();
    return this.observableState();
  }

  updateProjectiles(deltaMs) {
    const seconds = deltaMs / 1000;
    for (const bullet of this.playerBullets) {
      bullet.y -= PLAYER_BULLET_SPEED * seconds;
    }
    for (const bullet of this.enemyBullets) {
      bullet.x += bullet.vx * seconds;
      bullet.y += bullet.vy * seconds;
    }
    this.playerBullets = this.playerBullets.filter((bullet) => bullet.y + bullet.height >= 0);
    this.enemyBullets = this.enemyBullets.filter(
      (bullet) =>
        bullet.y <= GAME_HEIGHT + bullet.height &&
        bullet.x >= -bullet.width &&
        bullet.x <= GAME_WIDTH + bullet.width,
    );
  }

  updateEnemies(deltaMs) {
    const seconds = deltaMs / 1000;
    for (const enemy of this.enemies) {
      enemy.y += ENEMY_SPEED * seconds;
      if (this.timeMs >= enemy.nextShotAtMs) {
        const diagonal = Number.parseInt(enemy.id.split("-").at(-1), 10) % 2 === 0;
        this.enemyBullets.push({
          id: `enemy-bullet-${this.nextEntityOrdinal++}`,
          x: enemy.x + enemy.width / 2 - 3,
          y: enemy.y + enemy.height,
          width: 6,
          height: 10,
          vx: diagonal ? (enemy.x < GAME_WIDTH / 2 ? 55 : -55) : 0,
          vy: ENEMY_BULLET_SPEED,
        });
        enemy.nextShotAtMs += 1300;
        this.emit("enemy_fired", { id: enemy.id, diagonal });
      }
    }
    const escaped = this.enemies.filter((enemy) => enemy.y > GAME_HEIGHT);
    for (const enemy of escaped) {
      this.loseLife("enemy_escaped");
      this.emit("enemy_escaped", { id: enemy.id });
    }
    this.enemies = this.enemies.filter((enemy) => enemy.y <= GAME_HEIGHT);
  }

  resolveCollisions() {
    const usedPlayerBullets = new Set();
    for (const bullet of this.playerBullets) {
      const enemy = this.enemies.find((candidate) => overlaps(bullet, candidate));
      if (enemy) {
        usedPlayerBullets.add(bullet.id);
        this.hitEnemy(enemy.id);
        continue;
      }
      if (this.boss && overlaps(bullet, this.boss)) {
        usedPlayerBullets.add(bullet.id);
        this.hitBoss();
      }
    }
    this.playerBullets = this.playerBullets.filter(
      (bullet) => !usedPlayerBullets.has(bullet.id),
    );

    const collidingEnemyBullet = this.enemyBullets.find((bullet) => overlaps(bullet, this.player));
    if (collidingEnemyBullet) {
      this.enemyBullets = this.enemyBullets.filter(
        (bullet) => bullet.id !== collidingEnemyBullet.id,
      );
      this.loseLife("enemy_bullet");
    }
    const collidingEnemy = this.enemies.find((enemy) => overlaps(enemy, this.player));
    if (collidingEnemy) {
      this.enemies = this.enemies.filter((enemy) => enemy.id !== collidingEnemy.id);
      this.loseLife("enemy_plane");
    }
  }

  observableState() {
    return {
      seed: this.seed,
      bossAfterMs: this.bossAfterMs,
      timeMs: Math.round(this.timeMs),
      paused: this.paused,
      phase: this.phase,
      lives: this.lives,
      score: this.score,
      player: { ...this.player },
      playerBulletCount: this.playerBullets.length,
      enemies: this.enemies.map((enemy) => ({
        id: enemy.id,
        type: enemy.type,
        x: Math.round(enemy.x * 1000) / 1000,
        y: Math.round(enemy.y * 1000) / 1000,
        hp: enemy.hp,
        visual: enemy.visual,
      })),
      enemyBulletCount: this.enemyBullets.length,
      boss: this.boss ? { ...this.boss } : null,
      events: this.events.map((event) => ({ ...event })),
    };
  }

  exportReplay() {
    return {
      schemaVersion: 1,
      seed: this.seed,
      bossAfterMs: this.bossAfterMs,
      frames: this.replayFrames.map((frame) => ({
        deltaMs: frame.deltaMs,
        input: { ...frame.input },
      })),
    };
  }

  static replay(recording) {
    if (!recording || recording.schemaVersion !== 1 || !Array.isArray(recording.frames)) {
      throw new Error("Unsupported replay recording");
    }
    const model = new GameModel({
      seed: recording.seed,
      bossAfterMs: recording.bossAfterMs,
    });
    for (const frame of recording.frames) {
      model.step(frame.deltaMs, frame.input, { record: false });
    }
    return model;
  }
}

function drawPlane(context, x, y, color, accent, scale = 1) {
  context.fillStyle = color;
  context.fillRect(x + 10 * scale, y, 8 * scale, 28 * scale);
  context.fillRect(x, y + 13 * scale, 28 * scale, 8 * scale);
  context.fillStyle = accent;
  context.fillRect(x + 12 * scale, y + 6 * scale, 4 * scale, 8 * scale);
}

function createBrowserGame() {
  const canvas = document.querySelector("#game");
  const context = canvas.getContext("2d");
  const parameters = new URLSearchParams(window.location.search);
  const seed = Number(parameters.get("seed") || 1337);
  const bossAfterMs = Number(parameters.get("bossAfter") || 180) * 1000;
  let model = new GameModel({ seed, bossAfterMs });
  let lastFrameAt = performance.now();
  const held = new Set();

  const lives = document.querySelector("#lives");
  const score = document.querySelector("#score");
  const bossHealth = document.querySelector("#boss-health");
  const status = document.querySelector("#status");
  const seedLabel = document.querySelector("#seed-label");
  seedLabel.textContent = `Seed ${seed}`;

  function render() {
    context.fillStyle = "#03050f";
    context.fillRect(0, 0, GAME_WIDTH, GAME_HEIGHT);
    for (let index = 0; index < 64; index += 1) {
      const x = (index * 83 + seed * 17) % GAME_WIDTH;
      const y = (index * 137 + Math.floor(model.timeMs / 18)) % GAME_HEIGHT;
      context.fillStyle = index % 7 === 0 ? "#65f7e5" : "#59618f";
      context.fillRect(x, y, index % 5 === 0 ? 2 : 1, index % 5 === 0 ? 2 : 1);
    }

    drawPlane(context, model.player.x, model.player.y, "#65f7e5", "#f7f4d2");
    for (const enemy of model.enemies) {
      drawPlane(context, enemy.x, enemy.y, enemy.color, enemy.accent);
    }
    context.fillStyle = "#f7f4d2";
    for (const bullet of model.playerBullets) {
      context.fillRect(bullet.x, bullet.y, bullet.width, bullet.height);
    }
    context.fillStyle = "#ffb347";
    for (const bullet of model.enemyBullets) {
      context.fillRect(bullet.x, bullet.y, bullet.width, bullet.height);
    }
    if (model.boss) {
      context.fillStyle = "#9a294b";
      context.fillRect(model.boss.x, model.boss.y, model.boss.width, model.boss.height);
      context.fillStyle = "#ffb347";
      context.fillRect(model.boss.x + 18, model.boss.y + 18, model.boss.width - 36, 18);
    }

    lives.textContent = String(model.lives);
    score.textContent = String(model.score).padStart(6, "0");
    bossHealth.textContent = model.boss ? String(model.boss.hp) : "--";
    status.textContent = model.paused
      ? "Paused"
      : model.phase === "playing"
        ? model.boss
          ? "Command carrier engaged"
          : "Defend the sector"
        : model.phase === "victory"
          ? "Sector secure"
          : "Mission failed";
  }

  function frame(now) {
    const delta = Math.min(50, now - lastFrameAt);
    lastFrameAt = now;
    model.step(delta, {
      left: held.has("ArrowLeft") || held.has("KeyA"),
      right: held.has("ArrowRight") || held.has("KeyD"),
      fire: held.has("Space"),
    });
    render();
    requestAnimationFrame(frame);
  }

  window.addEventListener("keydown", (event) => {
    if (["ArrowLeft", "ArrowRight", "Space"].includes(event.code)) {
      event.preventDefault();
    }
    if (event.code === "KeyP" && !event.repeat) {
      model.step(0, { pause: true });
    }
    held.add(event.code);
  });
  window.addEventListener("keyup", (event) => held.delete(event.code));
  document.querySelector("#start").addEventListener("click", () => {
    model = new GameModel({ seed, bossAfterMs });
    window.__AIT_GAME__.model = model;
  });
  document.querySelector("#pause").addEventListener("click", () => {
    model.step(0, { pause: true });
  });

  window.__AIT_GAME__ = {
    model,
    step: (deltaMs, input) => model.step(deltaMs, input),
    state: () => model.observableState(),
    exportReplay: () => model.exportReplay(),
  };
  render();
  requestAnimationFrame(frame);
}

if (typeof document !== "undefined") {
  createBrowserGame();
}
