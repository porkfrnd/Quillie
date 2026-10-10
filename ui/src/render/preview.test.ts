import { describe, it, expect } from 'vitest';
import {
  choosePath,
  isSimpleClip,
  frameUrl,
  QualityController,
  QUALITY_LADDER,
  type PreviewCapabilities,
} from './preview';
import { emptyProject, clipEnd, sequenceDuration, type Clip, type Sequence } from '$state/types';

function simpleClip(overrides: Partial<Clip> = {}): Clip {
  const base: Clip = {
    id: 'c1',
    media: 'm1',
    timeline_start: 0,
    source_in: 0,
    source_out: 120000,
    speed: { kind: 'static', value: 1 },
    transform: {
      x: { kind: 'static', value: 0 },
      y: { kind: 'static', value: 0 },
      scale_x: { kind: 'static', value: 1 },
      scale_y: { kind: 'static', value: 1 },
      rotation_deg: { kind: 'static', value: 0 },
      anchor_x: { kind: 'static', value: 0.5 },
      anchor_y: { kind: 'static', value: 0.5 },
    },
    opacity: { kind: 'static', value: 1 },
    audio: { gain_db: { kind: 'static', value: 0 }, fade_in: 0, fade_out: 0 },
    effects: [],
    link: null,
    group: null,
  };
  // The model is readonly, so variants are built by replacement rather than
  // mutation -- which also means the test cannot accidentally rely on a
  // halfway-mutated clip.
  return { ...base, ...overrides };
}

function seq(clips: Clip[]): Sequence {
  return {
    id: 's1',
    name: 'Main',
    resolution: [1920, 1080],
    tracks: [{ id: 't1', kind: 'Video', name: 'V1', clips, muted: false, locked: false, hidden: false, height_px: 80 }],
    transitions: [],
  };
}

const caps = (over: Partial<PreviewCapabilities> = {}): PreviewCapabilities => ({
  videoDecoder: false,
  softwareRenderer: false,
  webglVersion: 2,
  ...over,
});

describe('clip classification', () => {
  it('an untransformed clip is simple', () => {
    expect(isSimpleClip(simpleClip())).toBe(true);
  });

  it('a transformed clip is not simple', () => {
    const base = simpleClip();
    const c: Clip = {
      ...base,
      transform: { ...base.transform, x: { kind: 'static', value: 100 } },
    };
    expect(isSimpleClip(c)).toBe(false);
  });

  it('a speed change is not simple, because path A cannot resample', () => {
    const c = simpleClip({ speed: { kind: 'static', value: 2 } });
    expect(isSimpleClip(c)).toBe(false);
  });

  it('a generated clip with no media is not simple', () => {
    expect(isSimpleClip(simpleClip({ media: null }))).toBe(false);
  });

  it('a clip with effects is not simple', () => {
    const c = simpleClip({ effects: [{ id: 'e', effect: 'vignette', enabled: true, params: {} }] });
    expect(isSimpleClip(c)).toBe(false);
  });
});

function withOverlay(s: Sequence): Sequence {
  return {
    ...s,
    tracks: [
      ...s.tracks,
      {
        id: 't2',
        kind: 'Overlay',
        name: 'OV1',
        clips: [simpleClip({ id: 'c2' })],
        muted: false,
        locked: false,
        hidden: false,
        height_px: 60,
      },
    ],
  };
}

describe('path selection', () => {
  it('a single untransformed clip uses path A', () => {
    const p = choosePath(emptyProject(), seq([simpleClip()]), caps());
    expect(p.path).toBe('A');
    expect(p.reason).toBeTruthy();
  });

  it('an empty sequence still returns a usable path', () => {
    expect(choosePath(emptyProject(), seq([]), caps()).path).toBe('A');
  });

  it('two layers need compositing, so path A is out', () => {
    const p = choosePath(emptyProject(), withOverlay(seq([simpleClip()])), caps());
    expect(p.path).not.toBe('A');
  });

  it('a software renderer forces the host frame path', () => {
    // Pixi composites on the CPU under llvmpipe, so path A2 is slower than
    // decoding in the host, where the decode is native.
    const p = choosePath(emptyProject(), withOverlay(seq([simpleClip()])), caps({ softwareRenderer: true }));
    expect(p.path).toBe('B');
    expect(p.reason).toContain('software');
  });

  it('every decision carries a reason', () => {
    for (const c of [caps(), caps({ softwareRenderer: true }), caps({ videoDecoder: true })]) {
      expect(choosePath(emptyProject(), seq([simpleClip()]), c).reason.length).toBeGreaterThan(0);
    }
  });
});

describe('frame urls', () => {
  it('no path sends decoded rgba through the ipc boundary as json', () => {
    // All binary paths go through the custom URI scheme, not invoke().
    for (const path of ['A', 'A2', 'B', 'C'] as const) {
      const url = frameUrl(path, 'clip-1', 1234.5);
      expect(url).not.toContain('data:image');
      expect(url).not.toContain('base64');
      expect(url.startsWith('asset://') || url.startsWith('quillie://')).toBe(true);
    }
  });
});

describe('quality ladder', () => {
  function controller(budget = 16.7) {
    let t = 0;
    const c = new QualityController(budget, () => t);
    return { c, advance: (ms: number) => (t += ms) };
  }

  it('starts at full quality', () => {
    const { c } = controller();
    expect(c.current.scale).toBe(1);
    expect(c.isDraft).toBe(false);
  });

  it('steps down when frames run over budget', () => {
    const { c, advance } = controller(16.7);
    let changed = false;
    for (let i = 0; i < 40; i++) {
      advance(100);
      changed = c.record(30) || changed; // 30ms is well over budget
    }
    expect(changed).toBe(true);
    expect(c.current.scale).toBeLessThan(1);
    expect(c.isDraft).toBe(true);
  });

  it('does not oscillate: at most one step change per two seconds', () => {
    const { c, advance } = controller(16.7);
    let changes = 0;
    // Feed frames every 100ms for 10 seconds, all over budget.
    for (let i = 0; i < 100; i++) {
      advance(100);
      if (c.record(30)) changes++;
    }
    // 10s at one change per 2s is at most 5.
    expect(changes).toBeLessThanOrEqual(5);
    expect(changes).toBeGreaterThan(0);
  });

it('steps back up after a sustained good run', () => {
    const { c, advance } = controller(16.7);
    // Drive it down exactly one rung: over-budget frames until the first
    // step, then stop so the starting rung is known.
    for (let i = 0; i < 200 && c.current.scale === 1; i++) {
      advance(100);
      c.record(30);
    }
    const low = c.current.scale;
    expect(low).toBeLessThan(1);

    // Now excellent frames until the first step up.
    let steppedUp = false;
    for (let i = 0; i < 800 && !steppedUp; i++) {
      advance(100);
      steppedUp = c.record(2);
    }
    expect(steppedUp, 'a sustained good run must step quality back up').toBe(true);
    expect(c.current.scale).toBeGreaterThan(low);
  });

  it('the ladder is the documented one', () => {
    expect(QUALITY_LADDER.map((q) => q.scale)).toEqual([1, 0.75, 0.5, 0.33, 0.25]);
  });
});

describe('model mirroring', () => {
  it('clip end accounts for speed', () => {
    const fast = simpleClip({ speed: { kind: 'static', value: 2 } });
    // 120000 ticks of source at 2x occupies 60000 ticks.
    expect(clipEnd(fast)).toBe(60000);
    expect(clipEnd(simpleClip())).toBe(120000);
  });

  it('a keyframed speed falls back to 1x rather than producing NaN', () => {
    const c = simpleClip({ speed: { kind: 'keyframed', value: [] } });
    expect(clipEnd(c)).toBe(120000);
  });

  it('sequence duration is the furthest clip end', () => {
    const s = seq([simpleClip(), simpleClip({ id: 'c2', timeline_start: 500000 })]);
    expect(sequenceDuration(s)).toBe(620000);
  });
});