/**
 * Type declarations for `pixi.js/unsafe-eval`.
 *
 * Pixi ships an `index.d.ts` for this entry point, but its package `exports`
 * map does not route the `./unsafe-eval` subpath to that file under this
 * moduleResolution setting, so TypeScript falls back to "implicitly has an
 * 'any' type".
 *
 * This re-exports the real types rather than declaring `any`, because the
 * point of the file is to keep the strictness everywhere else: an `any` here
 * would silently un-type `Application`, `Graphics` and everything reached
 * through them.
 *
 * The runtime module is the CSP-safe Pixi build, which replaces the
 * `new Function` feature probe with explicit polyfills. See
 * `TimelineView.svelte` for why that matters here.
 */

declare module 'pixi.js/unsafe-eval' {
  export * from 'pixi.js';
}