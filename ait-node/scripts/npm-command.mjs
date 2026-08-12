import { spawnSync } from "node:child_process";
import path from "node:path";

export function npmInvocation(
  args,
  {
    platform = process.platform,
    execPath = process.execPath,
    npmExecPath = process.env.npm_execpath,
  } = {},
) {
  if (
    !Array.isArray(args) ||
    args.some((value) => typeof value !== "string" || value.includes("\u0000"))
  ) {
    throw new TypeError("npm arguments must be an array of NUL-free strings");
  }
  if (platform !== "win32") {
    return { command: "npm", args: [...args] };
  }

  const windowsPath = path.win32;
  const npmCli =
    typeof npmExecPath === "string" &&
    windowsPath.isAbsolute(npmExecPath) &&
    windowsPath.basename(npmExecPath).toLowerCase() === "npm-cli.js"
      ? npmExecPath
      : windowsPath.join(
          windowsPath.dirname(execPath),
          "node_modules",
          "npm",
          "bin",
          "npm-cli.js",
        );
  return {
    command: execPath,
    args: [npmCli, ...args],
  };
}

export function spawnNpmSync(args, options = {}) {
  const invocation = npmInvocation(args);
  return spawnSync(invocation.command, invocation.args, options);
}
