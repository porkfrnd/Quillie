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

  /**
   * Why the canvas is empty, if it is.
   *
   * A timeline that renders nothing and says nothing is indistinguishable
   * from a timeline that has nothing on it, and the second is a normal state
   * while the first is a bug. Showing the reason turns "the app is broken"
   * into a sentence the user or the log can act on.
   */
  let canvasError = $state<string | null>(null);

  /** Set after the first successful draw, so the empty state can say so. */
  let framesDrawn = $state(0);

  const flags = new DirtyFlags();
  let viewport = $state<Viewport>({
    scrollTicks: 0,
    widthPx: 800,
    pxPerTick: 0.002,
  });

  let gate: RenderGate | null = null;
  let destroyed = false;
  /**
   * Pixi teardown, captured from `mountCanvas` because it is created after an
   * await. Declared at component scope so the component's own cleanup can
   * reach it: a leaked WebGL context survives the view being closed.
   */
  let teardown: (() => void) | undefined;

  onMount(() => {
    // Svelte 5's onMount does not accept an async function that returns a
    // teardown, so the dynamic import and Pixi init live in `mountCanvas`
    // and the teardown is captured at component scope for the return below.

    void (async () => {
      try {
        await mountCanvas();
      } catch (e) {
        // Swallowed promises turn a renderer failure into a blank panel with
        // no trace. The message names the failure rather than assuming a
        // cause we have not established.
        canvasError = e instanceof Error ? e.message : String(e);
        console.error('[timeline] canvas failed to start', e);
      }
    })();

    return () => {
      destroyed = true;
      gate?.stop();
      teardown?.();
    };
  });

  async function mountCanvas(): Promise<void> {
      // Captured once: `host` is bound by Svelte and TS cannot narrow it
      // across the await, so the closure would otherwise see `T | null`.
      const el = host;
      if (!el) return;

      // Pixi probes for optional GPU features with `new Function`, so the CSP in
// tauri.conf.json allows 'unsafe-eval' for scripts.
//
// The CSP-safe `pixi.js/unsafe-eval` build was tried first, so the policy
// could stay strict. It loads, but its polyfill path throws "undefined is not
// a constructor" in this webview, which is a worse outcome than a documented
// policy: a blank timeline with no usable fallback.
//
// What 'unsafe-eval' does and does not buy an attacker: it permits eval of
// script that is already same-origin. It does not permit loading remote
// script, and `default-src 'self'` still blocks every external origin. The
// app loads no remote content, so the surface it opens is small -- and it is
// confined to the app's own origin, which is already trusted to run.
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
      // Pixi sizes the canvas to `el`, but the parent sets `contain: strict`,
      // which in turn constrains the canvas unless it is positioned to fill
      // that parent. Without this the canvas is laid out at zero height and
      // every rectangle is drawn outside the visible area.
      app.canvas.style.position = 'absolute';
      app.canvas.style.inset = '0';
      app.canvas.style.width = '100%';
      app.canvas.style.height = '100%';
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
          framesDrawn = layout.drawnClips;

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

      // A resize changes which clips are visible and where, so the content
      // layer is dirty too. This also covers the first frame: on mount the
      // element can still be zero-height, and without this the timeline stays
      // blank until something else happens to request a redraw.
      let lastWidth = app.screen.width;
      let lastHeight = app.screen.height;
      const onResize = (): void => {
        if (app.screen.width === lastWidth && app.screen.height === lastHeight) return;
        lastWidth = app.screen.width;
        lastHeight = app.screen.height;
        viewport = { ...viewport, widthPx: app.screen.width };
        flags.markAll();
        gate?.request();
      };

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
      const resizeObserver = new ResizeObserver(onResize);
      resizeObserver.observe(el);

      teardown = () => {
        gate?.stop();
        resizeObserver.disconnect();
        app.canvas.removeEventListener('wheel', onWheel);
        app.canvas.removeEventListener('pointerdown', onPointerDown);
        app.destroy(true, { children: true });
      };
  }
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

  {#if canvasError}
    <p class="diagnostic" role="alert">Timeline could not start: {canvasError}</p>
  {:else if framesDrawn === 0 && clips.length > 0}
    <p class="diagnostic">Nothing in view — scroll or zoom to see the clips.</p>
  {/if}
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

  .diagnostic {
    position: absolute;
    inset: auto var(--space-3) var(--space-3) var(--space-3);
    margin: 0;
    padding: var(--space-2) var(--space-3);
    font-size: 0.8125rem;
    color: var(--fg-muted);
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    /* The canvas sits underneath; the message has to be readable over it. */
    pointer-events: none;
  }
</style>