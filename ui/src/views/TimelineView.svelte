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
  import { media, pickMediaFiles, QuillieError } from '$ipc/commands';
  import { buildEmptyProject, buildMockProject, isMockMode, mockVariant } from '$state/mock';
  import {
    HEADER_WIDTH_PX,
    RULER_HEIGHT_PX,
    clampScrollY,
    clipRectInLane,
    timelineRects,
  } from '$render/rects';
  import { TRACK_FILL, UNRESOLVED_FILL } from '$render/layout';
  import { rulerTicks } from '$render/ruler';
  import { FPS_30, formatTimecode } from '$lib/time';

  let host: HTMLDivElement | null = $state(null);

  /** The playhead. Transient UI state, never part of the project model (§5). */
  let playhead = $state(0);

  /**
   * What is on the timeline.
   *
   * Starts empty on purpose. This used to be seeded with three clips so the
   * view was never blank -- and that is exactly the defect worth removing:
   * fake data in the shipped path, which shows a user an edit that does not
   * exist and cannot be undone. The empty state below now says what to do
   * instead, which is both honest and more useful than a decoy.
   *
   * These arrive from the open project once one exists.
   */
  let clips = $state<readonly SourceClip[]>([]);

  let trackOrder = $state<readonly string[]>([]);

  const flags = new DirtyFlags();
  let viewport = $state<Viewport>({
    scrollTicks: 0,
    widthPx: 800,
    pxPerTick: 0.002,
  });

  /** Viewport height in CSS pixels; drives the lane rects. */
  let viewportHeightPx = $state(0);
  let scrollY = $state(0);

  /**
   * Every rectangle in the view, from the one layout function.
   *
   * Derived rather than computed inside the Pixi draw loop, so the DOM
   * overlays and the canvas are reading the same numbers in the same frame.
   */
  const rects = $derived(
    timelineRects({
      viewportWidth: viewport.widthPx,
      viewportHeight: viewportHeightPx,
      trackIds: trackOrder,
      playheadTick: playhead,
      scrollTicks: viewport.scrollTicks,
      pxPerTick: viewport.pxPerTick,
      scrollY,
    }),
  );

  const laneRects = $derived(rects.lanes);

  /** Track display names. Falls back to the id so a lane is never blank. */
  function trackName(id: string): string {
    const fromClip = clips.find((c) => c.trackId === id);
    return fromClip?.name ?? id;
  }

  /**
   * Which icon the header row shows.
   *
   * Overlay and Caption are reported as themselves rather than flattened into
   * Video: a caption track rendered with a video icon tells the user nothing,
   * and these are two tracks in a real project.
   */
  function trackKind(id: string): 'Video' | 'Overlay' | 'Audio' | 'Caption' {
    return clips.find((c) => c.trackId === id)?.kind ?? 'Video';
  }

  const KIND_ICON: Record<string, string> = {
    Video: '▶',
    Overlay: '◈',
    Audio: '♪',
    Caption: 'CC',
  };

  /**
 * Whether a label is worth drawing, given how wide the clip is.
 *
 * Below roughly six characters there is no room for a name plus its ellipsis,
 * and a name truncated to two letters identifies nothing -- so the clip is
 * drawn unlabelled instead. An honest empty label beats a misleading one.
 */
function clipLabel(clip: SourceClip, widthPx: number): boolean {
  return widthPx >= 56;
}

/**
 * Shorten a name to fit, with an ellipsis.
 *
 * Character count rather than measured pixels, because the measurement is only
 * available once the Text exists -- and creating it to find out how wide it is
 * is the thing being avoided. 6px per character is the average advance for the
 * UI font at 11px, which is close enough for a fit test and never re-rasterises.
 */
function truncate(name: string, widthPx: number): string {
  const max = Math.max(3, Math.floor((widthPx - 12) / 6));
  return name.length <= max ? name : `${name.slice(0, max - 1)}…`;
}
  /**
   * The ruler's timecode.
   *
   * Delegates to `formatTimecode` in lib/time rather than reimplementing it.
   * The hand-rolled version here divided ticks by the frame *rate* instead of
   * by ticks-per-second and showed `0` where it should show `0:00` -- and the
   * mirror is already covered by unit tests, so reusing it is both shorter and
   * the version that is actually verified.
   */
  function timecode(ticks: number): string {
    return formatTimecode(ticks, FPS_30);
  }

  /** Set when the mock project should be zoomed to fit; see the effect below. */
  let fitToProject = false;

  /**
   * Browser-mode harness data, and only that.
   *
   * `isMockMode` reads `?mock=1` from the URL. A packaged build serves its
   * own scheme with no query string, so this is false there and the timeline
   * shows whatever the open project holds -- which may legitimately be
   * nothing.
   */
  if (isMockMode()) {
    const p = mockVariant() === 'empty' ? buildEmptyProject() : buildMockProject();
    clips = p.clips.map((c) => ({
      id: c.id,
      trackId: c.trackId,
      kind: c.kind,
      name: c.name,
      startTick: c.startTick,
      endTick: c.endTick,
      resolved: true,
      selected: false,
    }));
    trackOrder = p.tracks.map((t) => t.id);
    fitToProject = true;
  }

  // Fit the project to the window.
  //
  // The default zoom is tuned for a long timeline, so a 9-second mock project
  // overflowed and the third clip was correctly culled as off-screen --
  // indistinguishable, in a screenshot, from a clip that failed to draw.
  //
  // An effect rather than a one-off at init because the width is not known
  // until the element has been laid out; doing it inline read the placeholder
  // 800px and fitted to the wrong size.
  $effect(() => {
    if (!fitToProject) return;
    const usable = Math.max(200, viewport.widthPx - HEADER_WIDTH_PX - 24);
    const end = clips.reduce((max, c) => Math.max(max, c.endTick), 0);
    if (end <= 0 || usable <= 200) return;
    const next = usable / end;
    if (Math.abs(viewport.pxPerTick - next) > 1e-9) {
      viewport = { ...viewport, pxPerTick: next };
      flags.markAll();
      gate?.request();
    }
  });

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

  /**
   * The clip rectangles the canvas just drew, in canvas-local coordinates.
   *
   * Published so the browser-mode harness can check them against the DOM
   * ruler and header. A canvas has no inspectable geometry; without this the
   * harness could only assert on DOM elements, and the actual defect -- clips
   * drawn over the ruler -- lives entirely inside the canvas.
   */
  let drawnRects = $state<
    readonly { x: number; y: number; width: number; height: number }[]
  >([]);

  /** Why the last import failed, in plain language. */
  let importError = $state<string | null>(null);
  let importing = $state(false);

  /**
   * Import media and put it on the timeline.
   *
   * Goes through the real host command, so what appears is what ffprobe
   * actually read -- a file the user cannot decode produces a visible error
   * rather than a clip that is not really there.
   */
  async function onImportClick(): Promise<void> {
    if (importing) return;
    importing = true;
    importError = null;
    try {
      const picked = await pickMediaFiles();
      // A cancelled dialog is a normal outcome, not a failure to report.
      if (!picked) return;
      const outcome = await media.import(picked.split('\n'));

      if (outcome.imported.length === 0) {
        importError =
          outcome.failed?.join('; ') || 'Nothing could be imported from that file.';
        return;
      }

      // Place each import at the end of its own track, in selection order.
      const next: SourceClip[] = [...clips];
      const nextTracks: string[] = [...trackOrder];
      for (const m of outcome.imported) {
        const lane = m.width !== null && m.height !== null ? 'V1' : 'A1';
        if (!nextTracks.includes(lane)) nextTracks.push(lane);
        const onLane = next.filter((c) => c.trackId === lane);
        const laneEnd = onLane.reduce((max, c) => Math.max(max, c.endTick), 0);
        next.push({
          id: m.media_id,
          trackId: lane,
          kind: lane === 'V1' ? 'Video' : 'Audio',
          startTick: laneEnd,
          endTick: laneEnd + Math.round(m.duration_seconds * 120_000),
          resolved: true,
          selected: false,
        });
      }
      clips = next;
      trackOrder = nextTracks;
      flags.markAll();
      gate?.request();

      // Partial success still has to say what failed, or the user believes
      // they imported everything they picked.
      if (outcome.failed?.length) {
        importError = `Some files could not be imported: ${outcome.failed.join('; ')}`;
      }
    } catch (e) {
      importError =
        e instanceof QuillieError ? e.message : 'The import could not be completed.';
    } finally {
      importing = false;
    }
  }


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
const { Application, Container, Graphics, Text } = await import('pixi.js');

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
      const labelLayer = new Container();
      /**
       * Cached clip labels, keyed by text and width.
       *
       * Pixi rasterises text into a texture when it is created or when its
       * style changes. Rebuilding every label on every frame would re-rasterise
       * on every pointermove -- the easiest possible way to make dragging a
       * clip stutter on a floor machine.
       *
       * Deliberately a plain Map, not `SvelteMap`: it is written and read only
       * inside the draw closure and is never read during template rendering,
       * so Svelte reactivity would buy nothing and would put a proxy in the
       * one path that runs on every pointer event.
       */
      // eslint-disable-next-line svelte/prefer-svelte-reactivity
      const labelCache = new Map<
        string,
        InstanceType<typeof Text>
      >();
      app.stage.addChild(content, labelLayer, interaction, playheadLayer);

      const draw = (): void => {
        if (flags.consume(Layer.Content)) {
          content.clear();
          // Labels are their own container so clearing content does not destroy
          // the cached Text objects -- they are reused across frames.
          for (const t of labelCache.values()) t.visible = false;

          // Rects come from render/rects.ts -- the same function the DOM
          // header and lane rows read. The canvas previously used a separate
          // layout that started clips at y=4 while the ruler occupied 0..28,
          // which is how clips ended up drawn over the ruler's own labels.
          // One layout, one set of numbers, nothing to disagree about.
          const box = timelineRects({
            viewportWidth: app.screen.width,
            viewportHeight: app.screen.height,
            trackIds: trackOrder,
            playheadTick: playhead,
            scrollTicks: viewport.scrollTicks,
            pxPerTick: viewport.pxPerTick,
            scrollY,
          });

          let drawn = 0;
          const drawnRectsTmp: { x: number; y: number; width: number; height: number }[] = [];
          for (const lane of box.lanes) {
            for (const clip of clips) {
              if (clip.trackId !== lane.trackId) continue;
              const r = clipRectInLane(
                lane,
                clip.startTick,
                clip.endTick,
                viewport.scrollTicks,
                viewport.pxPerTick,
              );
              if (r.width <= 0 || r.right < lane.x || r.x > lane.x + lane.width) continue;
              const color = clip.resolved ? TRACK_FILL[clip.kind] : UNRESOLVED_FILL;
              drawn++;
              drawnRectsTmp.push({ x: r.x, y: r.y, width: Math.max(1, r.width), height: r.height });

              // A sub-pixel clip cannot honestly represent its duration, so it
              // is drawn as a hairline rather than a sliver that implies a
              // width it does not have.
              content
                .rect(r.x, r.y, Math.max(1, r.width), r.height)
                .fill({ color });
              if (clip.selected) {
                content.rect(r.x, r.y, Math.max(1, r.width), r.height).stroke({
                  width: 2,
                  color: 0xffffff,
                });
              }
              // A clip's label, drawn only when there is room to read it.
              //
              // One Text object per distinct label, kept in a Map and reused.
              // Building a Text every frame would re-rasterise the glyphs on
              // every redraw -- including every pointermove -- which is the
              // single easiest way to make a timeline stutter.
              const label = clipLabel(clip, r.width);
              if (label) {
                const key = `${clip.name ?? ''}|${Math.round(r.width)}`;
                let text = labelCache.get(key);
                if (!text) {
                  text = new Text({
                    text: truncate(clip.name ?? 'Clip', r.width),
                    style: {
                      fontSize: 11,
                      fill: 0xffffff,
                      fontFamily: 'system-ui, sans-serif',
                    },
                  });
                  labelCache.set(key, text);
                  labelLayer.addChild(text);
                }
                text.visible = true;
                text.x = r.x + 6;
                text.y = r.y + (r.height - text.height) / 2;
              }
            }
          }
          framesDrawn = drawn;
          // Published for the browser-mode harness and the debug overlay.
          //
          // A canvas has no DOM rects, so "do the clips cover the ruler?" is
          // otherwise unanswerable from outside the app -- which is how clips
          // sat over the ruler while every DOM check passed. Canvas-local
          // coordinates; the harness adds the canvas origin.
          //
          // Assigning a fresh array each frame is deliberate: Svelte compares
          // by reference, and mutating one in place would not notify the
          // data attribute.
          drawnRects = drawnRectsTmp;
        }

        // The ruler, drawn with the adaptive tick ladder from render/ruler.ts.
        // That module was written and unit-tested but never wired in, which
        // is why the user's screenshots show a bare strip with only a timecode
        // and no tick marks at all.
        if (flags.consume(Layer.Playhead)) {
          playheadLayer.clear();
          const box = timelineRects({
            viewportWidth: app.screen.width,
            viewportHeight: app.screen.height,
            trackIds: trackOrder,
            playheadTick: playhead,
            scrollTicks: viewport.scrollTicks,
            pxPerTick: viewport.pxPerTick,
            scrollY,
          });

          // Ticks are confined to the ruler band and start after the header
          // column, so they cannot be drawn over a track name.
          const rulerLeft = box.ruler.x + HEADER_WIDTH_PX;
          const rulerRight = box.ruler.x + box.ruler.width;
          for (const tick of rulerTicks({
            pxPerTick: viewport.pxPerTick,
            scrollTicks: viewport.scrollTicks,
            widthPx: box.ruler.width,
            frameRate: FPS_30,
          })) {
            const x = box.content.x + (tick.ticks - viewport.scrollTicks) * viewport.pxPerTick;
            if (x < rulerLeft - 1 || x > rulerRight) continue;
            const h = tick.major ? box.ruler.height * 0.55 : box.ruler.height * 0.28;
            playheadLayer
              .moveTo(x, box.ruler.y + box.ruler.height - h)
              .lineTo(x, box.ruler.y + box.ruler.height)
              .stroke({ width: 1, color: tick.major ? 0x6b6b78 : 0x3d3d47 });
          }

          // Playhead, across the content area only: a line from y=0 would cross
          // the ruler and the header column, which belong to neither.
          if (box.playheadX !== null) {
            playheadLayer
              .moveTo(box.playheadX, box.content.y)
              .lineTo(box.playheadX, box.content.y + box.content.height)
              .stroke({ width: 1, color: 0x6f9dff });
          }
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
        // The DOM lane rects read the element's own height, not the Pixi
        // screen height, so both are refreshed here. Reading only one is how
        // the header and the canvas end up disagreeing about where the
        // content starts.
        const h = el.clientHeight;
        if (app.screen.width === lastWidth && app.screen.height === lastHeight && h === viewportHeightPx) return;
        lastWidth = app.screen.width;
        lastHeight = app.screen.height;
        viewportHeightPx = h;
        viewport = { ...viewport, widthPx: app.screen.width };
        flags.markAll();
        gate?.request();
      };

      const onWheel = (e: WheelEvent): void => {
        e.preventDefault();
        if (e.ctrlKey) {
          viewport = zoomAt(viewport, e.deltaY < 0 ? 1.1 : 1 / 1.1, e.offsetX);
        } else if (e.shiftKey || e.deltaX !== 0) {
          viewport = { ...viewport, scrollTicks: viewport.scrollTicks + e.deltaX / viewport.pxPerTick };
        } else {
          // Plain vertical wheel scrolls the lanes.
          //
          // Clamped rather than allowed to go negative: an unclamped scroll
          // pushes the first lane up under the ruler, which is the overlap this
          // layout exists to prevent.
          scrollY = clampScrollY(scrollY + e.deltaY, rects.totalHeight, viewportHeightPx);
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
      // Seeded, because a ResizeObserver only fires on a *change*: an element
      // that mounts at its final size never triggers one, so without this the
      // viewport height stays 0 and every lane rect is empty. That is the
      // silent blank timeline, again, for the same reason.
      viewportHeightPx = el.clientHeight;
      onResize();

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
  data-drawn={JSON.stringify(drawnRects)}
>
  <!--
    The ruler and the track headers are DOM, not canvas: they carry the text a
    screen reader needs and they must stay selectable. Their rectangles come
    from render/rects.ts, the same place the clip lanes come from, so the two
    cannot disagree about where the ruler ends -- which is exactly how clips
    ended up drawn over the ruler's own labels.
  -->
  <div
    class="ruler"
    data-probe="ruler"
    style:height="{RULER_HEIGHT_PX}px"
  >
    <span class="timecode">{timecode(playhead)}</span>
  </div>

  <div
    class="header"
    data-probe="header"
    style:top="{RULER_HEIGHT_PX}px"
    style:width="{HEADER_WIDTH_PX}px"
  >
    {#each laneRects as lane (lane.trackId)}
      <div
        class="header-row"
        data-probe="lane"
        style:top="{lane.y - RULER_HEIGHT_PX}px"
        style:height="{lane.height}px"
      >
        <span class="track-kind" data-kind={trackKind(lane.trackId)} aria-hidden="true">
          {KIND_ICON[trackKind(lane.trackId)] ?? '▶'}
        </span>
        <span class="track-name">{trackName(lane.trackId)}</span>
      </div>
    {/each}
  </div>

  {#if canvasError}
    <p class="diagnostic" role="alert">Timeline could not start: {canvasError}</p>
  {:else if framesDrawn === 0 && clips.length > 0}
    <p class="diagnostic">Nothing in view — scroll or zoom to see the clips.</p>
  {:else if clips.length === 0}
    <!--
      The empty state. A blank void reads as "broken"; this reads as "nothing
      here yet, here is the one thing to do". The button dispatches the real
      import path rather than a placeholder, per the no-fake-controls rule.
    -->
    <div
      class="empty"
      data-probe="empty"
      style:top="{RULER_HEIGHT_PX}px"
      style:left="{HEADER_WIDTH_PX}px"
      style:right="0"
      style:bottom="0"
    >
      <p class="empty-title">No clips yet</p>
      <p class="empty-hint">Import a video or audio file to start editing.</p>
      <button type="button" class="import-btn" onclick={onImportClick}>
        Import media…
      </button>
      {#if importError}
        <p class="empty-error" role="alert">{importError}</p>
      {/if}
    </div>
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
    /* Height comes from the inline style bound to RULER_HEIGHT_PX, so this
       rule must not restate it. It used to say `height: 24px` while the layout
       said 28, and the two disagreed by exactly the amount clips overlapped
       the ruler. One number, from one place. */
    display: flex;
    align-items: center;
    padding: 0 var(--space-2);
    background: var(--bg-raised);
    border-bottom: 1px solid var(--border);
    /* The Pixi canvas sits under this; the ruler is a real DOM element so it
       stays selectable by assistive tech. */
    pointer-events: none;
  }

  /*
   * The track header column and its rows.
   *
   * Both are positioned absolutely and both take their coordinates from
   * rects.ts via inline styles. Positioning them by CSS instead is what let
   * the header drift out of step with the lanes the canvas draws.
   */
  .header {
    position: absolute;
    left: 0;
    bottom: 0;
    width: 148px;
    overflow: hidden;
    background: var(--bg-raised);
    border-right: 1px solid var(--border);
  }

  .header-row {
    position: absolute;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-2);
    font-size: var(--text-sm);
    color: var(--fg-muted);
    border-bottom: 1px solid var(--border);
  }

  .track-kind {
    flex: none;
    width: 1.2em;
    text-align: center;
    color: var(--fg-subtle);
  }

  .track-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  /*
   * The empty state sits below the ruler, in the content area, rather than
   * centred over the whole panel -- centring it would cover the ruler and
   * reintroduce the overlap this layout was rebuilt to remove.
   */
  .empty {
    position: absolute;
    /* Bound inline to RULER_HEIGHT_PX / HEADER_WIDTH_PX for the same reason
       the ruler's height is: a literal here would drift from the layout. */
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    text-align: center;
  }

  .empty-title {
    margin: 0;
    font-size: var(--text-md);
    color: var(--fg);
  }

  .empty-hint {
    margin: 0;
    color: var(--fg-muted);
  }

  .empty-error {
    margin: 0;
    max-width: 60ch;
    color: var(--danger);
  }

  .import-btn {
    appearance: none;
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--accent-fg);
    font: inherit;
    cursor: pointer;
  }

  .import-btn:hover {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .import-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>