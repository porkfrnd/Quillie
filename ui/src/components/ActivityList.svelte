<!--
  The Activity list (§17).

  One place showing job state, progress and cancel. A job never blocks the UI
  with a modal — the user must always be able to keep editing while a proxy
  builds.

  Progress bars carry a text percentage alongside the fill, because colour and
  position alone are not sufficient feedback and the list has to stay readable
  for screen readers.
-->

<script lang="ts">
  interface Job {
    readonly id: string;
    readonly kind: string;
    /** 0..1 */
    readonly progress: number;
    readonly state: 'Queued' | 'Running' | 'Done' | 'Failed' | 'Cancelled';
  }

  let jobs = $state<readonly Job[]>([]);

  // Props are how the shell feeds real jobs in; the placeholder keeps the
  // component renderable before the first job event arrives.
  let { initial = [] as readonly Job[] }: { initial?: readonly Job[] } = $props();

  $effect(() => {
    jobs = initial;
  });

  function cancel(id: string): void {
    // Cancellation must reach the scheduler, not just hide the row (§9.1).
    void import('$ipc/commands').then((m) => m.jobs.cancel(id));
  }
</script>

{#if jobs.length === 0}
  <div class="activity empty">
    <span class="label">No background work</span>
  </div>
{:else}
  <ul class="activity" aria-label="Background activity">
    {#each jobs as job (job.id)}
      <li class="job" data-state={job.state.toLowerCase()}>
        <span class="kind">{job.kind}</span>

        <div
          class="meter"
          role="progressbar"
          aria-valuenow={Math.round(job.progress * 100)}
          aria-valuemin="0"
          aria-valuemax="100"
          aria-label={`${job.kind} progress`}
        >
          <div class="fill" style:width={`${Math.round(job.progress * 100)}%`}></div>
        </div>

        <!-- Text, not just a bar. -->
        <span class="pct">{Math.round(job.progress * 100)}%</span>

        {#if job.state === 'Running' || job.state === 'Queued'}
          <button type="button" class="cancel" onclick={() => cancel(job.id)}>
            Cancel
          </button>
        {:else}
          <span class="state">{job.state}</span>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  .activity {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: 100%;
    margin: 0;
    padding: 0 var(--space-3);
    list-style: none;
    font-size: var(--text-sm);
    overflow-x: auto;
  }

  .empty {
    color: var(--fg-subtle);
  }

  .job {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    white-space: nowrap;
  }

  .kind {
    color: var(--fg-muted);
  }

  .meter {
    width: 96px;
    height: 4px;
    background: var(--bg-sunken);
    border-radius: 2px;
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    transition: width var(--duration-fast);
  }

  .job[data-state='failed'] .fill {
    background: var(--danger);
  }

  .pct,
  .state {
    color: var(--fg-subtle);
    font-variant-numeric: tabular-nums;
    min-width: 32px;
  }

  .cancel {
    appearance: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    padding: 0 var(--space-2);
    cursor: pointer;
  }

  .cancel:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
</style>