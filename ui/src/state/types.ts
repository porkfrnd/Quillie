/**
 * Project types mirroring the Rust model (§6.2).
 *
 * These are the UI's view of the model. They are a mirror, not a second source
 * of truth: the Rust `Project` is authoritative and the UI holds a copy built
 * from patches. The shapes here must match what serde emits, which is why
 * `Animatable` uses adjacent tagging (`{ kind, value }`) rather than the
 * internally-tagged form serde cannot represent for a float.
 *
 * No time value is ever a float. Ticks are integers at 1/120000 s.
 */

export type Tick = number;

export interface FrameRate {
  readonly num: number;
  readonly den: number;
}

export type TrackKind = 'Video' | 'Overlay' | 'Audio' | 'Caption';

/** A constant, keyframed, or driven value. */
export type Animatable<T> =
  | { kind: 'static'; value: T }
  | { kind: 'keyframed'; value: readonly Keyframe<T>[] }
  | { kind: 'driven'; value: { source: unknown; map: unknown } };

export interface Keyframe<T> {
  readonly time: Tick;
  readonly value: T;
  readonly interp: unknown;
}

export interface Transform {
  readonly x: Animatable<number>;
  readonly y: Animatable<number>;
  readonly scale_x: Animatable<number>;
  readonly scale_y: Animatable<number>;
  readonly rotation_deg: Animatable<number>;
  readonly anchor_x: Animatable<number>;
  readonly anchor_y: Animatable<number>;
}

export interface EffectInstance {
  readonly id: string;
  readonly effect: string;
  readonly enabled: boolean;
  readonly params: Readonly<Record<string, unknown>>;
}

export interface Clip {
  readonly id: string;
  readonly media: string | null;
  readonly timeline_start: Tick;
  readonly source_in: Tick;
  readonly source_out: Tick;
  readonly speed: Animatable<number>;
  readonly transform: Transform;
  readonly opacity: Animatable<number>;
  readonly audio: {
    readonly gain_db: Animatable<number>;
    readonly fade_in: Tick;
    readonly fade_out: Tick;
  };
  readonly effects: readonly EffectInstance[];
  readonly link: string | null;
  readonly group: string | null;
}

export interface Track {
  readonly id: string;
  readonly kind: TrackKind;
  readonly name: string;
  readonly clips: readonly Clip[];
  readonly muted: boolean;
  readonly locked: boolean;
  readonly hidden: boolean;
  readonly height_px: number;
}

export interface Sequence {
  readonly id: string;
  readonly name: string;
  readonly resolution: readonly [number, number];
  readonly tracks: readonly Track[];
  readonly transitions: readonly unknown[];
}

export interface MediaRef {
  readonly id: string;
  readonly path: string;
  readonly duration: Tick;
  readonly rotation: number;
  readonly vfr: boolean;
}

export interface Project {
  readonly schema_version: number;
  readonly id: string;
  readonly name: string;
  readonly frame_rate: FrameRate;
  readonly resolution: readonly [number, number];
  readonly sample_rate: number;
  readonly media: Readonly<Record<string, MediaRef>>;
  readonly sequences: readonly Sequence[];
  readonly captions: readonly unknown[];
  readonly markers: readonly Marker[];
}

export interface Marker {
  readonly id: string;
  readonly time: Tick;
  readonly kind: 'Note' | 'Chapter' | 'Beat' | 'Scene' | 'Silence';
  readonly label: string;
}

/** Everything after the playhead, computed once per project. */
export function clipEnd(clip: Clip): Tick {
  const sourceDuration = Math.max(0, clip.source_out - clip.source_in);
  const speed = clip.speed.kind === 'static' ? (clip.speed.value ?? 1) : 1;
  const safeSpeed = Math.max(speed, Number.MIN_VALUE);
  return clip.timeline_start + Math.round(sourceDuration / safeSpeed);
}

/** The combined length of every track. */
export function sequenceDuration(sequence: Sequence): Tick {
  let max = 0;
  for (const track of sequence.tracks) {
    for (const clip of track.clips) {
      max = Math.max(max, clipEnd(clip));
    }
  }
  return max;
}

/** An empty project, used before one is opened. */
export function emptyProject(): Project {
  return {
    schema_version: 1,
    id: '00000000-0000-0000-0000-000000000000',
    name: 'Untitled',
    frame_rate: { num: 30, den: 1 },
    resolution: [1920, 1080],
    sample_rate: 48000,
    media: {},
    sequences: [
      { id: '00000000-0000-0000-0000-000000000001', name: 'Main', resolution: [1920, 1080], tracks: [], transitions: [] },
    ],
    captions: [],
    markers: [],
  };
}