/**
 * Preview path A: a single untransformed clip through a `<video>` element
 * (§8.2).
 *
 * # Why this path exists
 *
 * It is the cheapest way to show a moving picture: the WebView decodes, the
 * compositor draws, and Quillie contributes nothing per frame. No frame
 * crosses the IPC boundary, no texture is uploaded, no PixiJS pass runs.
 *
 * On a floor machine this is often the *only* path that holds 30 fps, which
 * is why it is the default rather than a fallback.
 *
 * # When it is not usable
 *
 * Path A cannot composite, cannot transform, and cannot play a clip at a
 * different speed than the file. The selection logic in `choosePath` encodes
 * those limits rather than discovering them at playback time.
 *
 * The limits are checked against the *project*, not the current frame, because
 * a path that is valid at 10 s and invalid at 20 s would produce a visible
 * change mid-playback, which is worse than switching paths once up front.
 */

import type { Animatable, Clip, Project, Sequence } from '$state/types';

/** The four preview paths (§8.2). */
export type PreviewPath = 'A' | 'A2' | 'B' | 'C';

export interface PathDecision {
  readonly path: PreviewPath;
  /** Why. Surfaced in diagnostics so a path switch is never mysterious. */
  readonly reason: string;
}

export interface PreviewCapabilities {
  /** WebCodecs is present and not software-rendered. */
  readonly videoDecoder: boolean;
  readonly softwareRenderer: boolean;
  readonly webglVersion: number;
}

/** Can this clip play through a plain `<video>` element? */
export function isSimpleClip(clip: Clip): boolean {
  return (
    clip.media !== null &&
    // A speed change needs frame resampling, which path A cannot do. The
    // *value* matters, not just the variant: a static 2x is still a speed
    // change, and treating it as simple makes the element play the wrong
    // duration.
    isUnity(clip.speed) &&
    // Transforms are not composited by the element.
    isIdentityTransform(clip) &&
    // Opacity below 1 needs a compositing pass.
    isUnity(clip.opacity) &&
    // Effects need a shader.
    clip.effects.length === 0
  );
}

/** Is this a plain constant equal to `expected`? */
function isConst(v: Animatable<number>, expected: number, epsilon = 1e-6): boolean {
  return v.kind === 'static' && Math.abs(v.value - expected) < epsilon;
}

function isUnity(v: Animatable<number>): boolean {
  return isConst(v, 1);
}

function isZero(v: Animatable<number>): boolean {
  return isConst(v, 0);
}

function isIdentityTransform(clip: Clip): boolean {
  const t = clip.transform;
  return (
    isZero(t.x) &&
    isZero(t.y) &&
    isUnity(t.scale_x) &&
    isUnity(t.scale_y) &&
    isZero(t.rotation_deg) &&
    // Anchor at the centre is the default and does not move the picture.
    isConst(t.anchor_x, 0.5) &&
    isConst(t.anchor_y, 0.5)
  );
}

/**
 * Choose a preview path for a whole sequence (§8.2).
 *
 * Decided per *segment* in the document; in practice a per-project decision
 * that is stable across time is better, because a path that changes as the
 * playhead moves is visible as a flicker nobody can explain.
 *
 * The default order is A, then A2, then C, then B.
 */
export function choosePath(
  project: Project,
  sequence: Sequence,
  caps: PreviewCapabilities,
): PathDecision {
  const clips = sequence.tracks.flatMap((t) => t.clips);

  if (clips.length === 0) {
    return { path: 'A', reason: 'nothing to preview' };
  }

  const allSimple = clips.every(isSimpleClip);
  const singleLayer = sequence.tracks.filter((t) => t.kind !== 'Audio').length <= 1;

  if (allSimple && singleLayer) {
    return { path: 'A', reason: 'one untransformed layer plays natively in the WebView' };
  }

  // Path A2: up to three video elements as Pixi textures. The WebView still
  // decodes; Pixi composites.
  //
  // Skipped entirely on a software renderer: Pixi's compositing cost there is
  // paid on the CPU, and two layers is already enough to miss frame. Falling
  // through to the host frame path is faster because that decode is native.
  const videoLayers = sequence.tracks.filter((t) => t.kind === 'Video' || t.kind === 'Overlay').length;
  if (videoLayers <= 3 && caps.webglVersion >= 1 && !caps.softwareRenderer) {
    return {
      path: 'A2',
      reason: `${videoLayers} layers need compositing; the WebView decodes and Pixi composites`,
    };
  }

  // Path C: WebCodecs. Avoids frame transfer over IPC, but availability varies
  // by WebView version and a software decoder is too slow to be worth it.
  if (caps.videoDecoder && !caps.softwareRenderer) {
    return { path: 'C', reason: 'WebCodecs avoids a frame copy per displayed frame' };
  }

  // Path B is the floor: the host decodes and serves frames. It works
  // everywhere and is the only option with no preconditions.
  return {
    path: 'B',
    reason:
      caps.softwareRenderer
        ? 'software renderer: the host frame path is the only one that can keep up'
        : 'four or more layers exceeds what WebView textures handle smoothly',
  };
}

/**
 * Where a decoded frame comes from, for the given path.
 *
 * Path A uses a `<video>` element and a blob/media URL; the others request a
 * frame by ID from the host (§5 rule 6 — never decoded RGBA through JSON).
 */
export function frameUrl(path: PreviewPath, clipId: string, sourceTimeTicks: number): string {
  switch (path) {
    case 'A':
      return `asset://localhost/preview/${clipId}`;
    case 'A2':
    case 'C':
      return `quillie://resource/frame-${clipId}-${Math.round(sourceTimeTicks)}`;
    case 'B':
      return `quillie://resource/frame-${clipId}-${Math.round(sourceTimeTicks)}`;
  }
}

/**
 * Draft-then-refine (§8.3): the immediate frame while the playhead moves, the
 * full-quality one once it settles.
 *
 * The delay is 120 ms because that is where the perceived boundary sits: below
 * it a user reads the result as "the editor is behind"; above it they read it
 * as "it settled", which is correct.
 */
export const REFINE_DELAY_MS = 120;

export interface QualityStep {
  readonly scale: number;
  readonly label: string;
}

/** The preview quality ladder (§8.3). */
export const QUALITY_LADDER: readonly QualityStep[] = [
  { scale: 1.0, label: 'Full' },
  { scale: 0.75, label: 'High' },
  { scale: 0.5, label: 'Medium' },
  { scale: 0.33, label: 'Low' },
  { scale: 0.25, label: 'Draft' },
] as const;

/**
 * The quality ladder controller (§8.3).
 *
 * Steps down when p90 frame time exceeds 1.15x budget, up when p90 stays under
 * 0.6x for 90 frames, and never changes more than once every 2 s. The last
 * rule is the one that stops oscillation, and it is why a project sitting near
 * the budget is stable instead of flickering between two levels.
 */
export class QualityController {
  private samples: number[] = [];
  private index = 0;
  private lastChangeMs = 0;
  private goodFrames = 0;

  constructor(
    private readonly frameBudgetMs: number,
    private readonly now: () => number = () => Date.now(),
  ) {}

  get current(): QualityStep {
    return QUALITY_LADDER[this.index] ?? QUALITY_LADDER[0]!;
  }

  /** True when below full quality, which the UI shows as a "Draft" label. */
  get isDraft(): boolean {
    return this.current.scale < 1;
  }

  /** Report one frame's time. Returns true when the level changed. */
  record(frameMs: number): boolean {
    this.samples.push(frameMs);
    // p90 over the last 30 frames, per the document.
    if (this.samples.length > 30) this.samples.shift();
    if (this.samples.length < 30) return false;

    const sorted = [...this.samples].sort((a, b) => a - b);
    const p90 = sorted[Math.floor(sorted.length * 0.9)]!;

    const now = this.now();
    if (now - this.lastChangeMs < 2000) return false;

    if (p90 > this.frameBudgetMs * 1.15) {
      this.goodFrames = 0;
      if (this.index < QUALITY_LADDER.length - 1) {
        this.index++;
        this.lastChangeMs = now;
        this.samples = [];
        return true;
      }
      return false;
    }

    if (p90 < this.frameBudgetMs * 0.6) {
      this.goodFrames++;
      if (this.goodFrames >= 90 && this.index > 0) {
        this.index--;
        this.lastChangeMs = now;
        this.samples = [];
        this.goodFrames = 0;
        return true;
      }
    } else {
      this.goodFrames = 0;
    }
    return false;
  }

  reset(): void {
    this.index = 0;
    this.samples = [];
    this.goodFrames = 0;
  }
}