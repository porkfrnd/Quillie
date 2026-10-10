<!--
  Quick Edit: a stepper over the same project (§17).

  Each step is a filtered view of the timeline and inspector, not a separate
  editing mode. Leaving Quick Edit leaves the project valid and fully editable
  -- there is no "quick mode" that has to be finalised.
-->

<script lang="ts">
  interface Step {
    readonly id: string;
    readonly label: string;
    readonly done: boolean;
  }

  const steps = $state<readonly Step[]>([
    { id: 'import', label: 'Import', done: false },
    { id: 'trim', label: 'Remove sections', done: false },
    { id: 'music', label: 'Add music', done: false },
    { id: 'captions', label: 'Generate captions', done: false },
    { id: 'title', label: 'Pick a title style', done: false },
    { id: 'motion', label: 'Apply motion', done: false },
    { id: 'export', label: 'Export', done: false },
  ]);

  let active = $state(0);
</script>

<div class="quick">
  <ol class="steps" aria-label="Quick Edit steps">
    {#each steps as step, i (step.id)}
      <li>
        <button
          type="button"
          class:active={i === active}
          class:done={step.done}
          aria-current={i === active ? 'step' : undefined}
          onclick={() => (active = i)}
        >
          <!-- Text marker, not colour alone. -->
          <span class="marker" aria-hidden="true">{step.done ? '✓' : String(i + 1)}</span>
          {step.label}
        </button>
      </li>
    {/each}
  </ol>

  <section class="stage">
    <h2>{steps[active]?.label}</h2>
    <p class="hint">
      Each step is a filtered view of the timeline. Nothing is applied
      automatically — every change proposes ops you can undo as one step.
    </p>
  </section>
</div>

<style>
  .quick {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .steps {
    width: 240px;
    margin: 0;
    padding: var(--space-3) 0;
    list-style: none;
    background: var(--bg-sunken);
    border-right: 1px solid var(--border);
  }

  .steps button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
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

  .steps button:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .steps button.active {
    border-left-color: var(--accent);
    background: var(--accent-subtle);
    color: var(--fg);
  }

  .marker {
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border: 1px solid var(--border-strong);
    border-radius: 50%;
    font-size: 11px;
    flex: none;
  }

  .done .marker {
    border-color: var(--heat-ok);
    color: var(--heat-ok);
  }

  .stage {
    flex: 1;
    padding: var(--space-6);
    overflow: auto;
  }

  .stage h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-lg);
    font-weight: 600;
  }

  .hint {
    color: var(--fg-muted);
    max-width: 52ch;
  }
</style>