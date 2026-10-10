import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath, URL } from 'node:url';

// §14.1 requires the first screen to load only the shell. Quick Edit,
// Timeline and Motion Studio are separate lazy chunks, and PixiJS loads when
// the first preview or timeline needs it -- not at startup.
//
// The manualChunks below is what enforces that. Without it Rollup will happily
// pull PixiJS into the entry chunk because the timeline imports it, and the
// 250 KB shell budget turns into a multi-megabyte one.
export default defineConfig({
  plugins: [svelte()],

  resolve: {
    alias: {
      $lib: fileURLToPath(new URL('./src/lib', import.meta.url)),
      $state: fileURLToPath(new URL('./src/state', import.meta.url)),
      $render: fileURLToPath(new URL('./src/render', import.meta.url)),
      $ipc: fileURLToPath(new URL('./src/ipc', import.meta.url)),
      $motion: fileURLToPath(new URL('./src/motion-eval', import.meta.url)),
    },
  },

  // Tauri serves the dev build from a fixed port and needs a predictable host.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**', '**/target/**'] },
  },

  build: {
    // Tauri v2 targets webview2 and webkitgtk, both of which handle modern
    // syntax. esnext keeps the output small and avoids a downlevel pass.
    target: 'esnext',
    sourcemap: false,
    minify: 'esbuild',
    // §14.6 budget. These are enforced in CI, not aspirational.
    chunkSizeWarningLimit: 250,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/pixi.js')) return 'pixi';
          if (id.includes('node_modules')) return 'vendor';
          return undefined;
        },
      },
    },
  },

  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});