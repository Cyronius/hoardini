import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

const ui = (p: string) => fileURLToPath(new URL(`../ui/${p}`, import.meta.url));

export default defineConfig({
  root: fileURLToPath(new URL(".", import.meta.url)),
  resolve: {
    // One React copy: the component under test lives in ui/, the harness in specs/.
    alias: {
      "@ui": ui("src"),
      "react-dom": ui("node_modules/react-dom"),
      react: ui("node_modules/react"),
    },
    dedupe: ["react", "react-dom"],
  },
  server: { fs: { allow: [fileURLToPath(new URL("..", import.meta.url))] } },
  test: {
    environment: "jsdom",
    include: ["ui/tests/**/*.test.ts"],
    server: { deps: { inline: [/streamdown/, /@streamdown/] } },
  },
});
