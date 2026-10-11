//! The undo stack (§6.3).
//!
//! Holds ops with their inverses, capped at 1000 entries or 8 MB of serialized
//! size, whichever comes first. The cap is a memory budget, not a user-facing
//! "you can only undo 1000 steps" promise: nothing that fits in the cap is
//! refused, and the oldest entries fall off the bottom first, which is how
//! every other editor behaves.
//!
//! The stack lives in `quillie-core`, not the host, because its policy (what
//! coalesces, what the cap is, what undo returns) is pure logic that the
//! property tests should own. The host only calls it.

use super::edit::{EditOp, Inverse};

/// The entry cap (§6.3).
const MAX_ENTRIES: usize = 1000;
/// The serialized-size cap (§6.3), 8 MB.
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// One undoable step: the op the user performed and the inverse that undoes
/// it. Redo is the same pair read from the other end, so nothing extra is
/// stored.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    op: EditOp,
    inverse: Inverse,
    /// Serialized size of the op, charged against the byte cap. Computed once
    /// at push time because measuring during eviction would mean re-serializing
    /// under pressure, which is exactly when the cost hurts most.
    op_bytes: usize,
}

fn serialized_size<T: serde::Serialize>(value: &T) -> usize {
    // A panic here would mean a type in the model lost Serialize, which the
    // model's own round-trip tests catch long before this runs.
    serde_json::to_vec(value).map(|v| v.len()).unwrap_or(0)
}

/// The undo/redo stack for one open project.
///
/// Continuous gestures coalesce by the caller (a drag commits one op on
/// release), so the stack never sees partial gestures and needs no timer.
#[derive(Debug, Default)]
pub struct UndoStack {
    /// Past ops, oldest first. `undo` pops from the end.
    past: Vec<Entry>,
    /// Undone ops, most recent first. `redo` pops from the end. Cleared by
    /// every new push, because branching history is a separate feature.
    future: Vec<Entry>,
    bytes: usize,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an op that was applied. Returns `false` when the op was a no-op
    /// (`Inverse::NoOp`) and so has nothing to undo, in which case it is not
    /// recorded: an undo step that does nothing is a step the user must press
    /// twice to get past, which reads as a dropped keystroke.
    pub fn push(&mut self, op: EditOp, inverse: Inverse) -> bool {
        if matches!(inverse, Inverse::NoOp) {
            return false;
        }
        // A new op makes every undone op unreachable through redo. Keeping
        // them would present a redo that conflicts with the current state.
        self.future.clear();
        let op_bytes = serialized_size(&op);
        self.past.push(Entry { op, inverse, op_bytes });
        self.bytes += op_bytes;
        self.evict();
        true
    }

    /// Drop oldest entries until both caps hold. The byte cap evicts
    /// whole entries, not partial ones: a half-remembered op cannot be undone.
    fn evict(&mut self) {
        while self.past.len() > 1 && (self.past.len() > MAX_ENTRIES || self.bytes > MAX_BYTES) {
            let dropped = self.past.remove(0);
            self.bytes = self.bytes.saturating_sub(dropped.op_bytes);
        }
    }

    /// Undo the most recent op, returning the inverse to apply. The op moves
    /// to the redo side, so `redo` can reapply it exactly.
    pub fn undo(&mut self) -> Option<Inverse> {
        let entry = self.past.pop()?;
        self.future.push(entry);
        self.future.last().map(|e| e.inverse.clone())
    }

    /// Redo the most recently undone op. The op itself is returned, not an
    /// inverse: redo applies the original op forward.
    pub fn redo(&mut self) -> Option<EditOp> {
        let entry = self.future.pop()?;
        self.past.push(entry);
        self.past.last().map(|e| e.op.clone())
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn len(&self) -> usize {
        self.past.len()
    }

    pub fn is_empty(&self) -> bool {
        self.past.is_empty()
    }

    /// Bytes currently charged against the cap. Exposed for tests and for the
    /// project health panel (§15.2), which reports undo journal weight.
    pub fn approx_bytes(&self) -> usize {
        self.bytes
    }

    /// Forget all history. Called when a project is opened or closed; a stack
    /// is only meaningful within one editing session of one project.
    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Clip, EditOp, Inverse, Project, Track, TrackKind};
    use crate::time::{FrameRate, Ticks};

    /// A project with one video track, for driving real ops through the stack.
    /// Returns the project plus its primary sequence and track ids, which
    /// `Project::new` generates rather than fixes.
    fn project() -> (Project, uuid::Uuid, uuid::Uuid) {
        let mut p = Project::new("test", FrameRate::new(30, 1), (1920, 1080));
        let seq = p.primary_sequence_mut();
        let track = Track::new(TrackKind::Video, "V1");
        let (seq_id, track_id) = (seq.id, track.id);
        seq.tracks.push(track);
        (p, seq_id, track_id)
    }

    fn insert_op(sequence: uuid::Uuid, track: uuid::Uuid, at: i64) -> EditOp {
        EditOp::InsertClip {
            sequence,
            track,
            index: 0,
            clip: Box::new(Clip::new(
                None,
                Ticks(at),
                Ticks(0),
                Ticks(at + 12_000),
            )),
        }
    }

    #[test]
    fn undo_restores_and_redo_reapplies() {
        let (mut p, seq, track) = project();
        let mut stack = UndoStack::new();
        let op = insert_op(seq, track, 0);
        let inverse = p.apply(op.clone()).unwrap();
        assert!(stack.push(op.clone(), inverse));

        let before = serde_json::to_string(&p).unwrap();
        let inv = stack.undo().unwrap();
        inv.undo(&mut p).unwrap();
        let after_undo = serde_json::to_string(&p).unwrap();
        assert_ne!(before, after_undo, "undo must change the project");

        let redo = stack.redo().unwrap();
        p.apply(redo).unwrap();
        let after_redo = serde_json::to_string(&p).unwrap();
        assert_eq!(before, after_redo, "redo must restore the state exactly");
    }

    #[test]
    fn a_new_op_clears_redo() {
        let (mut p, seq, track) = project();
        let mut stack = UndoStack::new();
        let op = insert_op(seq, track, 0);
        stack.push(op.clone(), p.apply(op).unwrap());
        stack.undo();
        assert!(stack.can_redo());
        // A second op, pushed with a hand-made (wrong, but undoable) inverse,
        // must still clear the redo branch.
        let op2 = insert_op(seq, track, 24_000);
        stack.push(
            op2,
            Inverse::UndoInsertClip {
                sequence: seq,
                track,
                clip: uuid::Uuid::new_v4(),
            },
        );
        assert!(!stack.can_redo(), "a new op must clear the redo branch");
    }

    #[test]
    fn noop_ops_are_not_recorded() {
        let mut stack = UndoStack::new();
        // An inverse of NoOp describes an op that changed nothing; recording
        // it would give undo a step that does nothing.
        assert!(!stack.push(
            EditOp::DeleteClip {
                sequence: uuid::Uuid::nil(),
                track: uuid::Uuid::nil(),
                clip: uuid::Uuid::new_v4(),
            },
            Inverse::NoOp,
        ));
        assert!(stack.is_empty());
    }

    #[test]
    fn the_entry_cap_evicts_the_oldest() {
        let (mut p, seq, track) = project();
        let mut stack = UndoStack::new();
        for i in 0..(MAX_ENTRIES + 50) {
            let op = insert_op(seq, track, i as i64 * 12_000);
            let inv = p.apply(op.clone()).unwrap();
            stack.push(op, inv);
        }
        assert_eq!(stack.len(), MAX_ENTRIES, "the stack holds at most the cap");
        // Undoing to the bottom succeeds without error; the 50 oldest entries
        // are gone, which is the documented cost of the cap.
        for _ in 0..MAX_ENTRIES {
            stack.undo();
        }
        assert!(!stack.can_undo());
    }

    #[test]
    fn clear_forgets_everything() {
        let (mut p, seq, track) = project();
        let mut stack = UndoStack::new();
        let op = insert_op(seq, track, 0);
        stack.push(op.clone(), p.apply(op).unwrap());
        stack.undo();
        stack.clear();
        assert!(!stack.can_undo());
        assert!(!stack.can_redo());
        assert_eq!(stack.approx_bytes(), 0);
    }
}
