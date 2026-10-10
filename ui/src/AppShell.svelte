/**
 * The window shell: chrome, view switching, and the Activity list.
 *
 * Every action routes through the command registry (`$state/commands.ts`), so
 * menus, the palette and tooltips cannot drift apart (§17).
 */

<script lang="ts">
  import type { ViewId } from './state/views';
  import { VIEWS } from './state/views';
  import ActivityList from './components/ActivityList.svelte';
  import type { Component } from 'svelte';

  let current = $state<ViewId>('timeline');

  // Views are code-split: each is a separate chunk loaded on first entry.
  // Nothing but the shell is in the entry bundle.
  const loaders: Record<ViewId, () => Promise<{ default: Component }>> = {
    quick: () => import('./views/QuickEdit.svelte'),
    timeline: () => import('./views/TimelineView.svelte'),
    motion: () => import('./views/MotionStudio.svelte'),
  };

  let LoadedView = $state<Component | null>(null);
  let loading = $state(false);

  async function switchView(id: ViewId): Promise<void> {
    if (id === current || loading) return;
    current = id;
    loading = true;
    try {
      const mod = await loaders[id]();
      LoadedView = mod.default;
    } finally {
      loading = false;
    }
  }
</script>

<div class="shell">
  <header class="titlebar">
    <nav class="views" aria-label="Editor views">
      {#each VIEWS as view (view.id)}
        <button
          type="button"
          class="view-button"
          class:active={current === view.id}
          aria-current={current === view.id ? 'page' : undefined}
          onclick={() => void switchView(view.id)}
        >
          {view.label}
        </button>
      {/each}
    </nav>

    <div class="spacer"></div>

    <!-- Draft indicator (§8.3). Shown whenever the preview is below full
         quality, with a text label rather than colour alone. -->
    <span class="draft" hidden title="Preview is below full quality">Draft</span>
  </header>

  <main class="workspace">
    {#if LoadedView && !loading}
      <LoadedView />
    {:else}
      <div class="loading" role="status">Loading…</div>
    {/if}
  </main>

  <footer class="statusbar">
    <ActivityList />
  </footer>
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--bg);
    color: var(--fg);
  }

  .titlebar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 40px;
    padding: 0 var(--space-3);
    background: var(--bg-raised);
    border-bottom: 1px solid var(--border);
  }

  .views {
    display: flex;
    gap: 2px;
  }

  .view-button {
    appearance: none;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    padding: var(--space-1) var(--space-3);
    cursor: pointer;
    transition:
      background var(--duration-fast),
      color var(--duration-fast);
  }

  .view-button:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .view-button.active {
    background: var(--accent-subtle);
    border-color: var(--accent);
    color: var(--accent);
  }

  .spacer {
    flex: 1;
  }

  .draft {
    font-size: var(--text-sm);
    color: var(--heat-draft);
    border: 1px solid currentColor;
    border-radius: var(--radius-sm);
    padding: 0 var(--space-1);
  }

  .workspace {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .loading {
    flex: 1;
    display: grid;
    place-items: center;
    color: var(--fg-subtle);
  }

  .statusbar {
    height: 28px;
    border-top: 1px solid var(--border);
    background: var(--bg-raised);
    overflow: hidden;
  }
</style>