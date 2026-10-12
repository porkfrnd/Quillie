<!--
  The Timeline view (Task 8).

  Owns the PixiJS canvas and hands drawing to the render layer, which keeps
  dirty-flag management and viewport maths out of the component. The component
  is responsible for things that are genuinely UI: mount/unmount, pointer
  events, and accessibility.

  PixiJS is imported here and nowhere else, so it lands in its own chunk and
  only loads when a timeline is first opened (§14.1).
-->

<script lang="ts">
  import { onMount } from 'svelte';
  import { DirtyFlags, Layer, RenderGate } from '$render/layers';
  import { layoutTimeline, hitTest, type SourceClip } from '$render/layout';
  import { zoomAt, xToTicks, snap, type Viewport } from '$render/viewport';
  import { FPS_30 } from '$lib/time';

  let host: HTMLDivElement | null = $state(null);

  /** The playhead. Transient UI state, never part of the project model (§5). */
  let playhead = $state(0);

  /**
   * What is on the timeline.
   *
   * Seeded with a short clip so the timeline is not empty on first open --
   * an editor whose centre pane is blank gives a user nothing to judge, and
   * the canvas dimensions are not known until the first layout. In the running
   * app these arrive from the project; here they exercise the real draw path.
   */
  let clips = $state<readonly SourceClip[]>([
    {
      id: 'seed-a',
      trackId: 'V1',
      kind: 'Video',
      startTick: 0,
      endTick: 120_000 * 4,
      resolved: true,
      selected: false,
    },
    {
      id: 'seed-b',
      trackId: 'V1',
      kind: 'Video',
      startTick: 120_000 * 5,
      endTick: 120_000 * 9,
      resolved: true,
      selected: false,
    },
    {
      id: 'seed-c',
      trackId: 'A1',
      kind: 'Audio',
      startTick: 0,
      endTick: 120_000 * 9,
      resolved: true,
      selected: false,
    },
  ]);

  let trackOrder = $state<readonly string[]>(['V1', 'A1']);

  const flags = new DirtyFlags();
  let viewport = $state<Viewport>({
    scrollTicks: 0,
    widthPx: 800,
    pxPerTick: 0.002,
  });

  let gate: RenderGate | null = null;
  let destroyed = false;

  onMount(() => {
    // Svelte 5's onMount does not accept an async function that returns a
    // teardown, so the dynamic import is wrapped and the teardown is captured
    // in a variable the outer function can return.
    let teardown: (() => void) | undefined;

    void (async () => {
      // Captured once: `host` is bound by Svelte and TS cannot narrow it
      // across the await, so the closure would otherwise see `T | null`.
      const el = host;
      if (!el) return;

      const { Application, Container, Graphics } = await import('pixi.js');

      const app = new Application();
      await app.init({
        // antialias off: the timeline is flat rectangles, and on a floor GPU
        // this is a measurable difference with no visible benefit.
        antialias: false,
        backgroundAlpha: 0,
        resizeTo: el,
        // Cap the device pixel ratio. A 4K display at dpr 3 quadruples the
        // fill cost for no legibility gain on 8px-tall clip rectangles.
        resolution: Math.min(globalThis.devicePixelRatio ?? 1, 2),
        autoDensity: true,
      });
      if (destroyed) {
        app.destroy(true);
        return;
      }
      app.canvas.setAttribute('aria-hidden', 'true');
      el.appendChild(app.canvas);

      // Three retained containers. Each redraws only when its flag is set.
      const content = new Graphics();
      const interaction = new Graphics();
      const playheadLayer = new Graphics();
      app.stage.addChild(content, interaction, playheadLayer);
      void Container;

      const draw = (): void => {
        if (flags.consume(Layer.Content)) {
          content.clear();

          // The layout decides which rectangles exist; this decides how they
          // look. Splitting them is what makes the geometry testable without
          // a GPU, which matters because the floor machine may have neither.
          const layout = layoutTimeline(clips, trackOrder, viewport, {
            canvasHeightPx: app.screen.height,
          });

          for (const row of layout.rows) {
            for (const c of row.clips) {
              // A sub-pixel rectangle cannot honestly represent a duration,
              // so it is drawn as a hairline marker instead of a sliver.
              if (c.tiny) {
                content.rect(c.x, c.y, 1, c.height).fill({ color: c.color });
                continue;
              }
              content.rect(c.x, c.y, c.width, c.height).fill({ color: c.color });
              if (c.selected) {
                content.rect(c.x, c.y, c.width, c.height).stroke({
                  width: 2,
                  color: 0xffffff,
                });
              }
            }
          }
        }
        if (flags.consume(Layer.Interaction)) interaction.clear();
        if (flags.consume(Layer.Playhead)) {
          playheadLayer.clear();
          const x = (playhead - viewport.scrollTicks) * viewport.pxPerTick;
          playheadLayer.moveTo(x, 0).lineTo(x, app.screen.height).stroke({ width: 1, color: 0x2f6feb });
        }
      };

      gate = new RenderGate(flags, draw);
      flags.markAll();
      gate.request();

      const onWheel = (e: WheelEvent): void => {
        e.preventDefault();
        if (e.ctrlKey) {
          viewport = zoomAt(viewport, e.deltaY < 0 ? 1.1 : 1 / 1.1, e.offsetX);
        } else {
          viewport = { ...viewport, scrollTicks: viewport.scrollTicks + e.deltaX / viewport.pxPerTick };
        }
        flags.markAll();
        gate?.request();
      };

      const onPointerDown = (e: PointerEvent): void => {
        const rect = el.getBoundingClientRect();
        const x = e.clientX - rect.left;
        const y = e.clientY - rect.top;

        // Select first, using the same layout the frame was drawn from. A
        // hit test that used a separately computed geometry would eventually
        // disagree with what is on screen, and the user would click a clip
        // and get a different one.
        const layout = layoutTimeline(clips, trackOrder, viewport, {
          canvasHeightPx: app.screen.height,
        });
        const hit = hitTest(layout, x, y);
        clips = clips.map((c) => ({ ...c, selected: hit !== null && c.id === hit.clipId }));

        // A click also moves the playhead and dirties exactly one layer.
        const raw = xToTicks(viewport, x);
        playhead = snap(raw, FPS_30, [], 0).ticks;
        flags.mark(Layer.Content);
        flags.mark(Layer.Playhead);
        gate?.request();
      };

      app.canvas.addEventListener('wheel', onWheel, { passive: false });
      app.canvas.addEventListener('pointerdown', onPointerDown);

      teardown = () => {
        gate?.stop();
        app.canvas.removeEventListener('wheel', onWheel);
        app.canvas.removeEventListener('pointerdown', onPointerDown);
        app.destroy(true, { children: true });
      };
    })();

    return () => {
      destroyed = true;
      gate?.stop();
      teardown?.();
    };
  });
</script>

<div
  class="timeline"
  role="grid"
  aria-label="Timeline"
  tabindex="0"
  bind:this={host}
>
  <div class="ruler" aria-hidden="true">
    <span class="timecode">{Math.floor(playhead / 120000)}s</span>
  </div>
</div>

<style>
  .timeline {
    position: relative;
    flex: 1;
    min-height: 0;
    /* §14.4: contain the layout and paint so a repaint here cannot trigger
       work in the panels beside it. */
    contain: strict;
    background: var(--bg-sunken);
    border-left: 1px solid var(--border);
  }

  .ruler {
    position: absolute;
    inset: 0 0 auto 0;
    height: 24px;
    display: flex;
    align-items: center;
    padding: 0 var(--space-2);
    background: var(--bg-raised);
    border-bottom: 1px solid var(--border);
    /* The Pixi canvas sits under this; the ruler is a real DOM element so it
       stays selectable by assistive tech. */
    pointer-events: none;
  }

  .timecode {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--fg-muted);
  }
</style>