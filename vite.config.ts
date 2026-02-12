import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // prevent vite from obscuring rust errors
  clearScreen: false,
  server: {
    // Tauri expects a fixed port
    strictPort: true,
  },
  // env variables with TAURI_ prefix are exposed to tauri's backend
  envPrefix: ["VITE_", "TAURI_"],
});
