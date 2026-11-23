/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // Vitest empties CSS imports. Keep ?raw ones, which tests read as text.
  test: { css: { include: [/\.css\?raw$/] } },
});
