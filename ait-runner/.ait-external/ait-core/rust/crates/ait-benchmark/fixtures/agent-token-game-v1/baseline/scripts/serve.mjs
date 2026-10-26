import { createReadStream } from "node:fs";
import { lstat, realpath } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = await realpath(resolve(dirname(fileURLToPath(import.meta.url)), ".."));
const port = Number(process.env.PORT || 4173);
const contentTypes = new Map([
  [".css", "text/css; charset=utf-8"],
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".mjs", "text/javascript; charset=utf-8"],
  [".svg", "image/svg+xml"],
]);

const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url || "/", "http://127.0.0.1");
    const requested = decodeURIComponent(url.pathname === "/" ? "/index.html" : url.pathname);
    const candidate = resolve(root, `.${requested}`);
    if (candidate !== root && !candidate.startsWith(`${root}${sep}`)) {
      response.writeHead(403).end("forbidden");
      return;
    }
    const canonical = await realpath(candidate);
    if (canonical !== root && !canonical.startsWith(`${root}${sep}`)) {
      response.writeHead(403).end("forbidden");
      return;
    }
    const metadata = await lstat(canonical);
    if (!metadata.isFile() || metadata.isSymbolicLink()) {
      response.writeHead(404).end("not found");
      return;
    }
    response.writeHead(200, {
      "cache-control": "no-store",
      "content-type": contentTypes.get(extname(canonical)) || "application/octet-stream",
      "x-content-type-options": "nosniff",
    });
    createReadStream(canonical).pipe(response);
  } catch {
    response.writeHead(404).end("not found");
  }
});

server.listen(port, "127.0.0.1", () => {
  process.stdout.write(`Starline Defender: http://127.0.0.1:${port}\n`);
});
