import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Separate from vite.config.ts on purpose: that file is tailored for Tauri
// dev/build (fixed port, HMR host, ignoring src-tauri/) and none of that
// applies to running Vitest. Keeping them apart avoids the test runner
// picking up Tauri-only server config or vice versa.
export default defineConfig({
  plugins: [vue()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.{test,spec}.{ts,js}"],
    globals: false,
    // Milestone A installs the runner; Milestone G writes the actual specs.
    // Vitest's default (exit 1 on zero test files) would otherwise break
    // `just test-unit` / `just check-all` / the future pre-commit test tier
    // for the entire gap between the two milestones.
    passWithNoTests: true,
  },
});
