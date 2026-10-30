// Test repositories carry their own commands and ignore rules. The installed
// CLI receives ordinary files and never selects behavior from the language.
import { createHash } from "node:crypto";
import { chmodSync, lstatSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";

const crlf = Buffer.from("native path with spaces and naïve text\r\nsecond line\r\n");
const binary = Buffer.from([0, 255, 13, 10, 128, 0, 42]);
const common = [
  { name: "data/space name/naïve.txt", bytes: crlf, mode: 0o644 },
  { name: "data/bytes.bin", bytes: binary, mode: 0o644 },
  { name: "scripts/entry.sh", bytes: Buffer.from("#!/bin/sh\nexit 0\n"), mode: 0o755 },
];

const repositories = {
  rust: {
    toolchains: [["rustc", "--version"], ["cargo", "--version"]],
    files: [
      { name: "Cargo.toml", bytes: Buffer.from('[package]\nname = "ait_language_fixture"\nversion = "0.1.0"\nedition = "2021"\n'), mode: 0o644 },
      { name: "src/lib.rs", bytes: Buffer.from('pub fn answer() -> u8 { 42 }\n#[cfg(test)] mod tests { #[test] fn native_files() { assert_eq!(super::answer(), 42); assert!(std::fs::read("data/space name/naïve.txt").unwrap().ends_with(b"second line\\r\\n")); assert_eq!(std::fs::read("data/bytes.bin").unwrap(), [0,255,13,10,128,0,42]); } }\n'), mode: 0o644 },
    ],
    build: ["cargo", "build", "--offline", "--target-dir", "build/rust"],
    test: ["cargo", "test", "--offline", "--target-dir", "build/rust"],
    ignore: "build/\nCargo.lock\n",
  },
  typescript_node: {
    toolchains: [["node", "--version"]],
    files: [
      { name: "package.json", bytes: Buffer.from('{"name":"ait-language-fixture","private":true,"type":"module"}\n'), mode: 0o644 },
      { name: "src/main.ts", bytes: Buffer.from('import { mkdirSync, readFileSync, writeFileSync } from "node:fs";\nconst answer: number = 42;\nif (process.argv[2] === "build") { mkdirSync("build/node", { recursive: true }); writeFileSync("build/node/result.txt", String(answer)); }\nelse if (process.argv[2] === "test") { if (readFileSync("build/node/result.txt", "utf8") !== "42" || !readFileSync("data/space name/naïve.txt").equals(Buffer.from("native path with spaces and naïve text\\r\\nsecond line\\r\\n")) || !readFileSync("data/bytes.bin").equals(Buffer.from([0,255,13,10,128,0,42]))) process.exit(1); }\nelse process.exit(2);\n'), mode: 0o644 },
    ],
    build: ["node", "--experimental-strip-types", "src/main.ts", "build"],
    test: ["node", "--experimental-strip-types", "src/main.ts", "test"],
    ignore: "build/\n",
  },
  go: {
    toolchains: [["go", "version"]],
    files: [
      { name: "go.mod", bytes: Buffer.from("module example.com/ait-language-fixture\n\ngo 1.20\n"), mode: 0o644 },
      { name: "cmd/app/main.go", bytes: Buffer.from('package main\nfunc main() {}\n'), mode: 0o644 },
      { name: "cmd/app/main_test.go", bytes: Buffer.from('package main\nimport (\n "bytes"\n "os"\n "testing"\n)\nfunc TestNativeFiles(t *testing.T) {\n a,e:=os.ReadFile("../../data/space name/naïve.txt")\n if e!=nil || !bytes.HasSuffix(a,[]byte("second line\\r\\n")) {t.Fatal(e)}\n b,e:=os.ReadFile("../../data/bytes.bin")\n if e!=nil || !bytes.Equal(b,[]byte{0,255,13,10,128,0,42}) {t.Fatal(e)}\n}\n'), mode: 0o644 },
    ],
    build: ["go", "build", "-o", "build/go-app", "./cmd/app"],
    test: ["go", "test", "./..."],
    ignore: "build/\n",
  },
  java: {
    toolchains: [["javac", "-version"], ["java", "-version"]],
    files: [
      { name: "src/Main.java", bytes: Buffer.from('import java.nio.file.*; import java.util.*;\npublic class Main { public static void main(String[] args) throws Exception { byte[] text=Files.readAllBytes(Paths.get("data", "space name", "naïve.txt")); byte[] bytes=Files.readAllBytes(Paths.get("data", "bytes.bin")); if (!new String(text, java.nio.charset.StandardCharsets.UTF_8).endsWith("second line\\r\\n") || !Arrays.equals(bytes,new byte[]{0,(byte)255,13,10,(byte)128,0,42})) throw new AssertionError("native fixture bytes changed"); } }\n'), mode: 0o644 },
    ],
    build: ["javac", "-encoding", "UTF-8", "-d", "build/java", "src/Main.java"],
    test: ["java", "-cp", "build/java", "Main"],
    ignore: "build/\n",
  },
};

export const languageRepositoryNames = Object.freeze(Object.keys(repositories));

export function languageRepository(name) {
  const fixture = repositories[name];
  if (!fixture) throw new Error(`unknown language repository: ${name}`);
  const manifest = {
    language: name,
    toolchains: fixture.toolchains,
    build: fixture.build,
    test: fixture.test,
  };
  return {
    name,
    toolchains: fixture.toolchains,
    files: [
      ...common,
      ...fixture.files,
      { name: ".aitignore", bytes: Buffer.from(fixture.ignore), mode: 0o644 },
      { name: "qualification.json", bytes: Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`), mode: 0o644 },
    ],
  };
}

export function seedLanguageRepository(root, fixture) {
  for (const file of fixture.files) {
    const target = path.join(root, ...file.name.split("/"));
    mkdirSync(path.dirname(target), { recursive: true });
    writeFileSync(target, file.bytes, { mode: file.mode });
    if (process.platform !== "win32") chmodSync(target, file.mode);
  }
}

export function verifyLanguageRepository(root, fixture, snapshot = null) {
  for (const file of fixture.files) {
    const target = path.join(root, ...file.name.split("/"));
    const stat = lstatSync(target);
    if (!stat.isFile() || stat.isSymbolicLink() || !readFileSync(target).equals(file.bytes)) {
      throw new Error(`language repository bytes changed: ${fixture.name}/${file.name}`);
    }
    if (process.platform !== "win32" && (stat.mode & 0o111) !== (file.mode & 0o111)) {
      throw new Error(`language repository executable mode changed: ${fixture.name}/${file.name}`);
    }
    if (snapshot) {
      const digest = createHash("sha256").update(file.bytes).digest("hex");
      if (!snapshot.files.some((entry) => entry.path === file.name && entry.sha256 === digest)) {
        throw new Error(`language repository Snapshot omitted source: ${fixture.name}/${file.name}`);
      }
    }
  }
  if (snapshot && snapshot.files.some((entry) => entry.path === "Cargo.lock" || entry.path.startsWith("build/"))) {
    throw new Error(`language repository Snapshot retained generated output: ${fixture.name}`);
  }
}

function nodeWorkingDirectory(root, command) {
  // Node's relative main-entry loader cannot resolve this Win32 namespace
  // spelling. Adapt only its foreign-runtime cwd; Core retains the exact root.
  const prefix = "\\\\?\\";
  if (process.platform !== "win32" || command !== "node" ||
      !root.startsWith(prefix) || !/^[A-Za-z]:\\/.test(root.slice(prefix.length))) return root;
  const ordinary = root.slice(prefix.length);
  const originalDirectory = statSync(root, { bigint: true });
  const nodeDirectory = statSync(ordinary, { bigint: true });
  if (!originalDirectory.isDirectory() || !nodeDirectory.isDirectory() ||
      originalDirectory.ino === 0n || nodeDirectory.ino === 0n ||
      originalDirectory.dev !== nodeDirectory.dev || originalDirectory.ino !== nodeDirectory.ino) {
    throw new Error("Node language cwd does not preserve the physical worktree");
  }
  return ordinary;
}

export function runLanguageRepository(recorder, root, fixture) {
  const manifest = JSON.parse(readFileSync(path.join(root, "qualification.json"), "utf8"));
  if (manifest.language !== fixture.name || !["build", "test"].every((key) =>
    Array.isArray(manifest[key]) && manifest[key].length > 0 && manifest[key].every((part) => typeof part === "string" && part.length > 0)
  ) || !Array.isArray(manifest.toolchains) || manifest.toolchains.length !== fixture.toolchains.length ||
    manifest.toolchains.some((probe, index) => !Array.isArray(probe) || probe.length < 2 ||
      probe.some(part => typeof part !== "string" || part.length === 0) ||
      JSON.stringify(probe) !== JSON.stringify(fixture.toolchains[index]))) {
    throw new Error(`language repository command manifest is invalid: ${fixture.name}`);
  }
  const toolchains = [];
  for (const [command, ...args] of manifest.toolchains) {
    const result = recorder.run(command, args, {
      cwd: nodeWorkingDirectory(root, command), label: `${fixture.name} toolchain ${command} version`,
      env: { ...process.env, CARGO_NET_OFFLINE: "true", GOPROXY: "off", GOTOOLCHAIN: "local" },
    });
    const versionOutput = `${result?.stdout ?? ""}\n${result?.stderr ?? ""}`.trim();
    if (result?.status !== undefined && result.status !== 0 ||
        !versionOutput || !/[0-9]/.test(versionOutput)) {
      throw new Error(`language repository toolchain version is unavailable: ${fixture.name}/${command}`);
    }
    toolchains.push({ command, args, version_output: versionOutput });
  }
  mkdirSync(path.join(root, "build"), { recursive: true });
  for (const phase of ["build", "test"]) {
    const [command, ...args] = manifest[phase];
    recorder.run(command, args, {
      cwd: nodeWorkingDirectory(root, command),
      label: `${fixture.name} repository ${phase}`,
      env: { ...process.env, CARGO_NET_OFFLINE: "true", GOPROXY: "off", GOTOOLCHAIN: "local" },
    });
  }
  return toolchains;
}
