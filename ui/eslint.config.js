import js from '@eslint/js';
import ts from 'typescript-eslint';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';

export default [
  {
    ignores: ['dist/', 'node_modules/', '../target/', '../crates/*/target/'],
  },
  js.configs.recommended,
  ...ts.configs.recommended,
  // Typed linting. no-floating-promises is worth the extra compiler pass: a
  // dropped promise rejection is a bug the user sees as "nothing happened".
  ...ts.configs.recommendedTypeChecked,
  ...svelte.configs['flat/recommended'],
  {
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
      parserOptions: {
        projectService: true,
        // Svelte components are not a standard extension, so the type-aware
        // rules cannot find them without this.
        extraFileExtensions: ['.svelte'],
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      // A void-returning floating promise is a real bug in a UI: the rejection
      // disappears and the user sees nothing happen.
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/consistent-type-imports': 'error',
      // Unused code in a bundle-size-budgeted app is dead weight.
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
      eqeqeq: ['error', 'always', { null: 'ignore' }],
      'no-console': ['warn', { allow: ['warn', 'error'] }],
    },
  },
  {
    files: ['**/*.svelte'],
    languageOptions: {
      parserOptions: { parser: ts.parser, projectService: true },
    },
    rules: {
      // Svelte templates handle promise rejection through its own event
      // plumbing, and the type-aware rules cannot see through a dynamic
      // `import('pixi.js')` inside a component. svelte-check already covers
      // both properly, so these are off here rather than globally.
      '@typescript-eslint/no-floating-promises': 'off',
      '@typescript-eslint/no-unsafe-call': 'off',
      '@typescript-eslint/no-unsafe-assignment': 'off',
      '@typescript-eslint/no-unsafe-member-access': 'off',
    },
  },
  // Config files and the budget script are plain JS, outside the tsconfig
// project, so the type-aware rules cannot run on them.
{
  ...ts.configs.disableTypeChecked,
  files: ['*.config.js', '*.config.ts', 'scripts/**'],
},
{
  // The budget script's console output is its actual deliverable.
  files: ['scripts/**'],
  rules: { 'no-console': 'off' },
},
];