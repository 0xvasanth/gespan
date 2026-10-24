import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

const root = fileURLToPath(new URL(".", import.meta.url));

export default defineConfig({
  root,
  resolve: {
    alias: {
      "@": `${root}src`,
      react: `${root}node_modules/react`,
      "react-dom": `${root}node_modules/react-dom`,
    },
  },
  oxc: { jsx: { runtime: "automatic" } },
  // Vite only serves files inside the workspace root, which stops at mobile/.
  // The protocol tests in ../tests/mobile load from the repository root.
  server: { fs: { allow: [fileURLToPath(new URL("..", import.meta.url))] } },
  test: {
    environment: "happy-dom",
    // Protocol round trips need both the desktop and mobile dependency trees.
    include: [
      "tests/**/*.test.ts",
      "tests/**/*.test.tsx",
      "../tests/mobile/**/*.test.ts",
    ],
    maxWorkers: 1,
  },
});
