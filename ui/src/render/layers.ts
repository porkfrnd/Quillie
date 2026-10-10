/**
 * Dirty-flag layer management (§14.4).
 *
 * The timeline draws from retained layers, each with its own dirty flag.
 * A redraw touches only the layers whose inputs changed, and at most once per
 * display frame.
 *
 * This is the single biggest reason the UI stays responsive on a floor
 * machine: a playhead move marks one layer dirty, not the whole timeline, so
 * a drag costs a redraw of a 2px-wide strip rather than every clip.
 *
 * The flags here are deliberately plain booleans in a mutable object rather
 * than reactive state. Reactive stores would allocate and schedule work on
 * every write, and these are written several times per pointer event.
 */

export const enum Layer {
  /** Clip rectangles, waveforms, thumbnails. */
  Content = 0,
  /** Selection outline and drag ghost. */
  Interaction = 1,
  /** The ruler, playhead and timecode. */
  Playhead = 2,
}

export const LAYER_COUNT = 3;

export class DirtyFlags {
  private readonly dirty = new Uint8Array(LAYER_COUNT);

  mark(layer: Layer): void {
    this.dirty[layer] = 1;
  }

  markAll(): void {
    this.dirty.fill(1);
  }

  isDirty(layer: Layer): boolean {
    return this.dirty[layer] === 1;
  }

  /**
   * Clear a layer's flag and report whether it needed drawing.
   *
   * Consume-on-read rather than clear-then-check: it makes "did this frame
   * need to draw?" and "mark it handled" a single atomic step, so a layer
   * cannot be marked dirty twice in one frame and silently skipped on the
   * second mark.
   */
  consume(layer: Layer): boolean {
    const was = this.dirty[layer] === 1;
    this.dirty[layer] = 0;
    return was;
  }

  anyDirty(): boolean {
    for (let i = 0; i < LAYER_COUNT; i++) {
      if (this.dirty[i]) return true;
    }
    return false;
  }

  clear(): void {
    this.dirty.fill(0);
  }
}

/**
 * The render loop gate.
 *
 * §8.6: when paused with nothing changing, no `requestAnimationFrame` loop
 * runs at all. It is not a loop that draws nothing — it is not running. On a
 * floor machine that is the difference between 0% idle CPU and a core pinned
 * by a timeline nobody is looking at.
 */
export class RenderGate {
  private rafId: number | null = null;
  private queued = false;

  constructor(
    private readonly flags: DirtyFlags,
    private readonly draw: () => void,
  ) {}

  /** Request a frame. Repeated calls within one frame collapse into one draw. */
  request(): void {
    if (this.queued) return;
    this.queued = true;
    this.rafId = requestAnimationFrame(() => {
      this.queued = false;
      this.rafId = null;
      this.draw();
    });
  }

  /** Stop the loop entirely. Safe to call when already stopped. */
  stop(): void {
    if (this.rafId !== null) {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
    this.queued = false;
    this.flags.clear();
  }

  get isRunning(): boolean {
    return this.rafId !== null;
  }
}