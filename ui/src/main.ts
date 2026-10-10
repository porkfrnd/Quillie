// Entry point.
//
// The shell is the only thing in the entry chunk. Everything else -- the three
// views, PixiJS -- is a dynamic import behind it, so the window paints fast
// even on a floor machine (§14.7).

import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';

const target = document.getElementById('app');
if (!target) {
  throw new Error('#app not found');
}

// Respect the OS preference on first run (§17). The user can override it in
// settings later; this is only the initial value.
if (!localStorage.getItem('quillie.theme')) {
  const prefersDark = globalThis.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false;
  document.documentElement.dataset['theme'] = prefersDark ? 'dark' : 'light';
}

export default mount(App, { target });