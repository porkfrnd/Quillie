import { defineConfig, mergeConfig } from 'vite';
import viteConfig from './vite.config';

/**
 * Vitest config, kept separate from vite.config.ts.
 *
 * Vitest 2 bundles its own copy of Vite 5 while this project uses Vite 6.
 * Putting the `test` block in vite.config.ts forces the two incompatible
 * `Plugin` type trees to meet and the typecheck fails on `plugins`. A separate
 * config with mergeConfig sidesteps it and keeps the app config clean.
 *
 * The aliases are merged from the app config on purpose: a test that resolves
 * `$lib` differently from the code under test is testing the wrong thing.
 */
export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: 'node',
      include: ['src/**/*.test.ts'],
    },
  }),
);