/**
 * Application shell.
 *
 * The shell is the only thing loaded at startup (§14.1). Quick Edit, Timeline
 * and Motion Studio are dynamic imports, so entering the app costs one small
 * chunk and nothing else. PixiJS is loaded by the timeline itself, when a
 * preview is first needed, rather than by the shell.
 *
 * §14.7: show the window shell within one second with a skeleton layout.
 * Everything below renders before any project data is fetched, because
 * fetching blocks the first paint.
 */

<script lang="ts">
  import { onMount } from 'svelte';
  import AppShell from './AppShell.svelte';

  let mounted = $state(false);

  onMount(() => {
    // Deferred one frame so the skeleton actually paints before heavier work
    // begins. Probing, calibration and recent-project thumbnails all happen
    // after this point (§14.7), never before it.
    requestAnimationFrame(() => {
      mounted = true;
    });
  });
</script>

{#if !mounted}
  <!-- Skeleton: the layout the user is about to see, at the right size, so
       the window does not visibly resize when the real content arrives. -->
  <div class="skeleton" role="status" aria-label="Loading Quillie">
    <div class="skeleton-bar"></div>
    <div class="skeleton-body">
      <div class="skeleton-panel"></div>
      <div class="skeleton-stage"></div>
    </div>
  </div>
{:else}
  <AppShell />
{/if}

<style>
  .skeleton {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--bg);
  }

  .skeleton-bar {
    height: 32px;
    background: var(--bg-raised);
    border-bottom: 1px solid var(--border);
  }

  .skeleton-body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .skeleton-panel {
    width: 240px;
    background: var(--bg-sunken);
    border-right: 1px solid var(--border);
  }

  .skeleton-stage {
    flex: 1;
    background: var(--bg);
  }
</style>