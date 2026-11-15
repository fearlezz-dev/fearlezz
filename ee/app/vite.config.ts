// The page of a work on the proof host: built into dist/, which the worker serves as its
// static assets (ee/host/wrangler.toml).
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  build: { outDir: "dist", emptyOutDir: true },
});
