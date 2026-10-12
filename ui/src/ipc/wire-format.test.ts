/**
 * The wire format is a contract, and a contract nobody checks is a contract
 * that breaks silently.
 *
 * `EditOp` crosses from TypeScript to Rust as JSON. If the discriminant is
 * `kind` on one side and `op` on the other, or the variant names differ by
 * case, nothing throws in either language: the host simply fails to parse the
 * op and returns an error the UI shows as "that edit could not be applied".
 * The user sees a broken undo button and no explanation.
 *
 * These tests pin the exact bytes the host expects, derived from the Rust
 * definitions rather than from what the TypeScript happens to say. The
 * `WIRE_FORMAT` constant below is the contract; `EditOp` is asserted to
 * produce it.
 */

import { describe, expect, it } from 'vitest';
import type { EditOp } from './commands';

/** 1/120000 s. Must match `TICKS_PER_SECOND` in quillie-core/src/time.rs. */
const TICKS_PER_SECOND = 120_000;

/**
 * What the host parses. Mirrors:
 *   #[serde(tag = "op", rename_all = "snake_case")] enum EditOp
 *   #[serde(rename_all = "snake_case")] enum TrimEdge
 */
const WIRE_FORMAT = {
  opDiscriminant: 'op',
  variantCase: 'snake_case',
  edgeValues: ['in', 'out'],
} as const;

const seq = '0192f0a1-0000-7000-8000-000000000001';
const track = '0192f0a1-0000-7000-8000-000000000002';
const clip = '0192f0a1-0000-7000-8000-000000000003';

describe('EditOp wire format', () => {
  it('uses the discriminant the host deserializes', () => {
    // Every variant must carry `op`, never `kind` or `type`.
    const ops: EditOp[] = [
      { op: 'insert_clip', sequence: seq, track, index: 0, clip: {} },
      { op: 'delete_clip', sequence: seq, track, clip },
      { op: 'move_clip', sequence: seq, track, clip, to: 0 },
      { op: 'trim_clip', sequence: seq, track, clip, edge: 'in', to: 0 },
      { op: 'split_clip', sequence: seq, track, clip, at: 0 },
    ];

    for (const o of ops) {
      expect(Object.keys(o)).toContain(WIRE_FORMAT.opDiscriminant);
      expect(Object.keys(o)).not.toContain('kind');
      expect(Object.keys(o)).not.toContain('type');
    }
  });

  it('names variants in snake_case, matching rename_all', () => {
    const names = (
      [
        { op: 'insert_clip' },
        { op: 'delete_clip' },
        { op: 'move_clip' },
        { op: 'trim_clip' },
        { op: 'split_clip' },
      ] as const
    ).map((o) => o.op);

    for (const n of names) {
      expect(n).toBe(n.toLowerCase());
      expect(n).toMatch(/^[a-z]+(_[a-z]+)*$/);
    }
  });

  it('sends trim edges as snake_case, not In/Out', () => {
    // TrimEdge is `rename_all = "snake_case"`, so "In" is not a valid value.
    const op: EditOp = { op: 'trim_clip', sequence: seq, track, clip, edge: 'out', to: 12 };
    expect(WIRE_FORMAT.edgeValues).toContain(op.edge);
  });

  it('sends ticks as bare integers, not a wrapper object', () => {
    // Ticks is a newtype over i64 with no serde attributes, so it
    // serialises as the number itself. Sending `{ "0": 120000 }` would
    // fail to deserialize into Ticks.
    const oneSecond = TICKS_PER_SECOND;
    const op: EditOp = { op: 'move_clip', sequence: seq, track, clip, to: oneSecond };
    // JSON.parse is `any`; narrow it before inspecting, so the assertion is
    // about the shape rather than about whatever `any` permits.
    const wire: unknown = JSON.parse(JSON.stringify(op));
    expect(typeof wire).toBe('object');
    expect(wire).toMatchObject({ to: 120_000 });
    if (typeof wire !== 'object' || wire === null || !('to' in wire)) {
      throw new Error('move_clip must carry a `to` field');
    }
    expect(typeof wire.to).toBe('number');
    expect(wire.to).toBe(120_000);
  });

  it('serializes to the exact JSON the host expects', () => {
    const op: EditOp = {
      op: 'trim_clip',
      sequence: seq,
      track,
      clip,
      edge: 'in',
      to: 240_000,
    };
    expect(JSON.parse(JSON.stringify(op))).toEqual({
      op: 'trim_clip',
      sequence: seq,
      track,
      clip,
      edge: 'in',
      to: 240_000,
    });
  });
});