// Builds the page, its worker and flowie-wasm (web/pkg, from scripts/build-web.sh) into
// web/dist, which `flowie-server serve --web web/dist` serves. `npm run dev` serves the sources
// and passes the sync API (/v1/) on to the server at FLOWIE_SERVER (default
// http://127.0.0.1:8787).
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";

const here = fileURLToPath(new URL(".", import.meta.url));

/** The app icon, from the repository's assets, as public/icon.png (git-ignored). */
function icon(): Plugin {
  return {
    name: "flowie-icon",
    configResolved() {
      mkdirSync(join(here, "public"), { recursive: true });
      copyFileSync(join(here, "../assets/icon.png"), join(here, "public/icon.png"));
    },
  };
}

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    return entry.isDirectory() ? files(path) : [path];
  });
}

/** sw.js, with the list of files the build wrote for it to keep, and a version from their contents. */
function serviceWorker(): Plugin {
  let outDir = "";
  return {
    name: "flowie-service-worker",
    apply: "build",
    configResolved(config) {
      outDir = config.build.outDir;
    },
    closeBundle: {
      order: "post",
      handler() {
        const written = files(outDir)
          .map((path) => relative(outDir, path).split(sep).join("/"))
          .filter((path) => path !== "sw.js" && !path.endsWith(".map"))
          .sort();
        const hash = createHash("sha256");
        for (const path of written) hash.update(path).update("\0").update(readFileSync(join(outDir, path)));
        const list = ["./", ...written.map((path) => `./${path}`)];
        const source = readFileSync(join(here, "sw.js"), "utf8")
          .replace("self.__FLOWIE_FILES__", JSON.stringify(list))
          .replace("self.__FLOWIE_VERSION__", JSON.stringify(hash.digest("hex").slice(0, 16)));
        writeFileSync(join(outDir, "sw.js"), source);
      },
    },
  };
}

export default defineConfig({
  // Relative URLs, so the build works from any path on the server.
  base: "./",
  plugins: [icon(), serviceWorker()],
  oxc: { jsx: { runtime: "automatic", importSource: "preact" } },
  // flowie-wasm's glue loads its .wasm with `new URL(…, import.meta.url)`, which needs an ES
  // module worker.
  worker: { format: "es", rolldownOptions: { output: { entryFileNames: "assets/worker-[hash].js" } } },
  build: { outDir: "dist", emptyOutDir: true, target: "es2022" },
  server: {
    proxy: { "/v1": { target: process.env.FLOWIE_SERVER ?? "http://127.0.0.1:8787", changeOrigin: true } },
  },
});
