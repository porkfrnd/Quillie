import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

export default {
  preprocess: vitePreprocess(),
  compilerOptions: {
    // Svelte 5 runes mode is used throughout; the legacy syntax would let a
    // component silently fall back to non-reactive behaviour.
    runes: true,
  },
};