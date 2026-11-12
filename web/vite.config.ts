import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// During development the UI runs on the Vite server and talks to a running
// backend (`cargo run -- serve`, port 8787 by default).
const backend = process.env.UNIGIF_BACKEND ?? 'http://127.0.0.1:8787';

export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: {
      '/api': backend,
      '/m': backend,
      '/healthz': backend,
      '/readyz': backend,
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
  },
});
