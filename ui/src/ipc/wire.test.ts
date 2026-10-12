/**
 * The wire payloads themselves.
 *
 * The command-name contract is checked in contract.test.ts. This checks the
 * *shape* each wrapper sends: that the right command name is invoked with the
 * right argument names.
 *
 * Argument names are the quiet failure. Tauri maps camelCase keys onto the
 * Rust parameter names, so a key that is merely mis-cased reaches the host as
 * `None` -- `export_video` would run at the wrong bitrate rather than fail.
 * Nothing in TypeScript can catch that, because the host's signature is not
 * visible from here, so it is pinned here instead.
 *
 * Every expected payload below is transcribed from the Rust command signature
 * in crates/quillie-app/src/commands.rs. When one changes, this fails.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { invoke } from '@tauri-apps/api/core';
import { editing, media, project, render } from './commands';

const mocked = vi.mocked(invoke);

beforeEach(() => {
  mocked.mockReset();
  mocked.mockResolvedValue(undefined);
});

afterEach(() => mocked.mockReset());

/** The last (command, args) pair sent across the boundary. */
function lastCall(): { command: string; args: Record<string, unknown> | undefined } {
  const call = mocked.mock.calls.at(-1);
  if (!call) throw new Error('invoke was never called');
  return { command: call[0], args: call[1] as Record<string, unknown> | undefined };
}

describe('project commands', () => {
  it('open sends the path', async () => {
    await project.open('/tmp/a.quillie');
    expect(lastCall()).toEqual({
      command: 'open_project',
      args: { path: '/tmp/a.quillie' },
    });
  });

  it('save sends no argument at all', async () => {
    // Regression: this used to send `{ id }`, which `save_project` has never
    // accepted. The Rust command takes only the app state.
    await project.save();
    expect(lastCall().command).toBe('save_project');
    expect(lastCall().args).toBeUndefined();
  });
});

describe('media commands', () => {
  it('import sends paths as a plain array', async () => {
    const paths = ['/a.mp4', '/b.mov'];
    await media.import(paths);

    const { command, args } = lastCall();
    expect(command).toBe('import_media');
    // `Vec<String>` needs an array; a readonly one is copied rather than
    // passed through so the host receives a plain JSON array.
    const sent = (args as { paths: unknown }).paths;
    expect(Array.isArray(sent)).toBe(true);
    expect(sent).toEqual(paths);
  });

  it('import does not mutate the caller array', async () => {
    const paths = ['/a.mp4'];
    await media.import(paths);
    expect(paths).toEqual(['/a.mp4']);
  });
});

describe('export command', () => {
  it('sends camelCase keys that map onto the Rust parameter names', async () => {
    // Rust: export_video(path, width, height, video_bitrate_bps, state)
    // Tauri converts videoBitrateBps -> video_bitrate_bps.
    await render.export('/out.mp4', 1920, 1080, 8_000_000);

    expect(lastCall()).toEqual({
      command: 'export_video',
      args: {
        path: '/out.mp4',
        width: 1920,
        height: 1080,
        videoBitrateBps: 8_000_000,
      },
    });
  });

  it('never sends a snake_case bitrate key', async () => {
    // The exact regression: a snake_case key deserialises to None and the
    // export silently runs at whatever default the preset carries.
    await render.export('/out.mp4', 640, 360, 1_000_000);
    const args = lastCall().args as Record<string, unknown>;
    expect(Object.keys(args)).not.toContain('video_bitrate_bps');
    expect(args.videoBitrateBps).toBe(1_000_000);
  });
});

describe('edit commands', () => {
  const seq = '0192f0a1-0000-7000-8000-000000000001';
  const track = '0192f0a1-0000-7000-8000-000000000002';
  const clip = '0192f0a1-0000-7000-8000-000000000003';

  it('apply forwards the op untouched', async () => {
    await editing.apply({ op: 'delete_clip', sequence: seq, track, clip });
    expect(lastCall()).toEqual({
      command: 'apply_edit',
      args: { op: { op: 'delete_clip', sequence: seq, track, clip } },
    });
  });

  it('undo and redo send no arguments', async () => {
    await editing.undo();
    expect(lastCall()).toEqual({ command: 'undo', args: undefined });

    await editing.redo();
    expect(lastCall()).toEqual({ command: 'redo', args: undefined });
  });

  it('a trim edge is sent snake_case, not as In/Out', async () => {
    // Rust: #[serde(rename_all = "snake_case")] on TrimEdge.
    await editing.apply({
      op: 'trim_clip',
      sequence: seq,
      track,
      clip,
      edge: 'out',
      to: 1,
    });
    const { args } = lastCall();
    expect((args as { op: { edge: string } }).op.edge).toBe('out');
  });

  it('ticks are sent as bare integers', async () => {
    // Rust: Ticks is a newtype over i64 with no serde attributes.
    await editing.apply({ op: 'move_clip', sequence: seq, track, clip, to: 240_000 });
    const { args } = lastCall();
    const to = (args as { op: { to: unknown } }).op.to;
    expect(typeof to).toBe('number');
    expect(to).toBe(240_000);
  });
});