// The page of a work on the proof host: built into dist/, which the worker serves as its
// static assets (ee/host/wrangler.toml).
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { type Plugin, defineConfig } from "vite";
import solid from "vite-plugin-solid";

const ee = fileURLToPath(new URL("..", import.meta.url));
const wasmDir = `${ee}target/wasm32-unknown-unknown/release/`;

/** Builds the comparison the page runs in its worker (ee/compare) to wasm before the page
 * builds or the dev server starts, so `fearlezz-page-compare.wasm?url` is always this source. */
function compareWasm(): Plugin {
  return {
    name: "fearlezz-page-compare",
    buildStart() {
      execFileSync("cargo", ["build", "--quiet", "--release", "--target", "wasm32-unknown-unknown", "-p", "fearlezz-page-compare"], {
        cwd: ee,
        stdio: "inherit",
      });
    },
  };
}

export default defineConfig({
  plugins: [compareWasm(), solid()],
  // A pattern, so the import's `?url` stays on the path.
  resolve: { alias: [{ find: /^fearlezz-page-compare\.wasm/, replacement: `${wasmDir}fearlezz_page_compare.wasm` }] },
  server: { fs: { allow: [".", wasmDir, "../../crates/fearlezz/tests/fixtures/compare"] } },
  worker: { format: "es" },
  build: { outDir: "dist", emptyOutDir: true },
});
