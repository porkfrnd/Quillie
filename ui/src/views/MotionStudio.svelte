<!--
  Motion Studio: tracking, animated text, subject-follow framing.

  Placeholder shell only. Per §24, placeholder UI for unbuilt features is not
  allowed, so this renders the frame with its real structure and no controls
  for things that do not exist yet. Controls appear when the features land,
  gated by the Feature Gate like everything else.
-->

<script lang="ts">
  const panels = [
    { id: 'tracker', label: 'Tracker' },
    { id: 'reframe', label: 'Subject follow' },
    { id: 'stabilise', label: 'Stabilization' },
    { id: 'recipes', label: 'Motion recipes' },
  ] as const;

  let selected = $state<(typeof panels)[number]['id']>('tracker');
</script>

<div class="motion">
  <nav class="rail" aria-label="Motion Studio panels">
    {#each panels as panel (panel.id)}
      <button
        type="button"
        class:active={selected === panel.id}
        aria-current={selected === panel.id ? 'page' : undefined}
        onclick={() => (selected = panel.id)}
      >
        {panel.label}
      </button>
    {/each}
  </nav>

  <section class="stage">
    <h2>{panels.find((p) => p.id === selected)?.label}</h2>
    <p class="status">Not built yet.</p>
  </section>
</div>

<style>
  .motion {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .rail {
    width: 200px;
    padding: var(--space-2) 0;
    background: var(--bg-sunken);
    border-right: 1px solid var(--border);
  }

  .rail button {
    display: block;
    width: 100%;
    appearance: none;
    border: 0;
    border-left: 2px solid transparent;
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    text-align: left;
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
  }

  .rail button:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .rail button.active {
    border-left-color: var(--accent);
    background: var(--accent-subtle);
    color: var(--fg);
  }

  .stage {
    flex: 1;
    padding: var(--space-6);
  }

  .stage h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-lg);
    font-weight: 600;
  }

  .status {
    color: var(--fg-subtle);
  }
</style>