//! Edit operations, inverses, and patches (§5 rule 2, §6.3).
//!
//! # The contract
//!
//! `Project::apply(op) -> Result<Inverse, EditError>` is the only way to change
//! a project. Every operation returns the state needed to undo it exactly, so
//! undo is total rather than approximate. Random op sequences followed by undo
//! must return the project to a byte-identical serialized state; there is a
//! property test for exactly that.
//!
//! `apply` deliberately returns **only** the inverse, because that is the
//! contract the rest of the system depends on. The patch the UI needs
//! ([`Patch`]) is derived from the op and its inverse by [`Patch::from`],
//! which is a pure function — it needs no stored state on the project, so
//! there is no way for a patch to drift out of sync with the model.

use super::model::{Clip, ClipId, Project, SequenceId, Track, TrackId};
use crate::time::Ticks;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("no sequence with id {0}")]
    UnknownSequence(SequenceId),
    #[error("no track with id {0}")]
    UnknownTrack(TrackId),
    #[error("no clip with id {0}")]
    UnknownClip(ClipId),
    #[error("trim would invert the clip: the new edge crosses the other one")]
    InvertedRange,
    #[error("track is locked")]
    TrackLocked,
}

/// A single change to the project.
///
/// Every variant is a *semantic* edit, not a field assignment. "Trim this clip"
/// is an op; "set `source_in` to 500" is not, because the first is something
/// undo can describe to a user and the second is not.
///
/// `InsertClip` embeds a whole `Clip`, which is far larger than the other
/// variants, so it is boxed. This keeps `EditOp` small enough to live on a
/// stack in the UI's hot path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Add a clip to a track at a position.
    InsertClip {
        sequence: SequenceId,
        track: TrackId,
        /// Position in the track's clip list, so insertion is order-stable.
        index: usize,
        clip: Box<Clip>,
    },
    /// Remove a clip. The inverse restores it at the same index.
    DeleteClip { sequence: SequenceId, track: TrackId, clip: ClipId },
    /// Change a clip's start time without changing its length.
    MoveClip { sequence: SequenceId, track: TrackId, clip: ClipId, to: Ticks },
    /// Change one edge of a clip.
    TrimClip {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        edge: TrimEdge,
        /// The new **source** offset for the trimmed edge.
        to: Ticks,
    },
    /// Cut a clip in two at a source offset.
    SplitClip { sequence: SequenceId, track: TrackId, clip: ClipId, at: Ticks },
    /// Delete a time range from a track and close the gap.
    RippleDelete { sequence: SequenceId, track: TrackId, range: TimeRange },
    /// Move a clip and shift everything after it, closing or opening a gap.
    RippleMove { sequence: SequenceId, track: TrackId, clip: ClipId, to: Ticks },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimEdge {
    /// Trim the start. Shifts `source_in` and moves `timeline_start` by the
    /// same amount so the visible content does not slide.
    In,
    /// Trim the end. Moves `source_out`.
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Ticks,
    pub end: Ticks,
}

impl TimeRange {
    pub fn new(start: Ticks, end: Ticks) -> Self {
        TimeRange { start, end }
    }
    pub fn duration(&self) -> Ticks {
        Ticks((self.end.raw() - self.start.raw()).max(0))
    }
}

/// The state needed to undo one [`EditOp`].
///
/// Inverses store whole removed values rather than diffs. That costs more
/// memory and buys something worth more: a diff has to be reapplied to state
/// that may have changed since, whereas a stored clip is restored verbatim.
/// Undo correctness is not worth optimising before it has ever been measured
/// to be a problem.
///
/// Sized deliberately: the whole-removal variants dominate, which is why the
/// small ones are boxed. Keeping `Inverse` under a few hundred bytes matters
/// because up to 1000 of these are live at once (the undo cap, §6.3).
///
/// # Naming
///
/// Each variant is named for **the operation it undoes**, not for the
/// restoration it performs. `Inverse::for_insert_clip` means "this undoes an
/// insert", and it deletes. Naming by effect instead is how the arms ended up
/// crossed the first time.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "inverse", rename_all = "snake_case")]
pub enum Inverse {
    NoOp,
    /// Undoes an insert: remove the clip that was added.
    UndoInsertClip { sequence: SequenceId, track: TrackId, clip: ClipId },
    /// Undoes a delete: put the removed clip back at its original index.
    UndoDeleteClip { sequence: SequenceId, track: TrackId, index: usize, clip: Box<Clip> },
    /// Undoes a move: restore the original start time.
    UndoMoveClip { clip: ClipId, from: Ticks },
    /// Undoes a trim: restore the original edge.
    UndoTrimClip { clip: ClipId, edge: TrimEdge, from: Ticks },
    /// Undoes a split by restoring the original clip verbatim, including its
    /// id, which is what makes undo byte-exact rather than merely visually
    /// equivalent.
    ///
    /// It stores the halves it created, not a rule for finding them. A later
    /// ripple move or drag can move the halves anywhere, so any positional
    /// search at undo time is a guess -- and a guess that fails only for
    /// certain edit orderings, which is the worst kind of bug.
    UndoSplitClip {
        sequence: SequenceId,
        track: TrackId,
        index: usize,
        original: Box<Clip>,
        halves: Vec<Clip>,
    },
    /// Undoes a ripple delete by restoring the whole prior clip list.
    UndoRippleDelete { sequence: SequenceId, track: TrackId, restored: Vec<Clip> },
    /// Undoes a ripple move: restore every shifted clip to where it was.
    UndoRippleMove { sequence: SequenceId, track: TrackId, from: Ticks, moved: Vec<(ClipId, Ticks)> },
}

impl Inverse {
    /// Undo this operation by restoring the exact prior state.
    ///
    /// Every arm either calls back into `apply` with the mirrored op, or
    /// restores a stored value directly. Direct restores are used where the
    /// mirrored op would not be exact (split restores the original id).
    pub fn undo(self, project: &mut Project) -> Result<(), EditError> {
        match self {
            Inverse::NoOp => Ok(()),
            Inverse::UndoInsertClip { sequence, track, clip } => {
                // Undo an insert by removing the inserted clip.
                project.apply(EditOp::DeleteClip { sequence, track, clip })?;
                Ok(())
            }
            Inverse::UndoDeleteClip { sequence, track, index, clip } => {
                // Undo a delete by re-inserting at the original index.
                project.apply(EditOp::InsertClip {
                    sequence,
                    track,
                    index,
                    clip,
                })?;
                Ok(())
            }
            Inverse::UndoMoveClip { clip, from } => {
                restore_start(project, clip, from)
            }
            Inverse::UndoTrimClip { clip, edge, from } => {
                restore_edge(project, clip, edge, from)
            }
            Inverse::UndoSplitClip { sequence, track, index, original, halves } => {
                // Remove exactly the halves this split created, wherever a
                // later edit moved them, then restore the original at `index`.
                let half_ids: Vec<ClipId> = halves.iter().map(|h| h.id).collect();
                let seq = project
                    .sequences
                    .iter_mut()
                    .find(|s| s.id == sequence)
                    .ok_or(EditError::UnknownSequence(sequence))?;
                let tr = seq
                    .track_mut(track)
                    .ok_or(EditError::UnknownTrack(track))?;
                let positions: Vec<usize> = (0..tr.clips.len())
                    .filter(|i| half_ids.contains(&tr.clips[*i].id))
                    .collect();
                for pos in positions.iter().rev() {
                    tr.clips.remove(*pos);
                }
                let idx = index.min(tr.clips.len());
                tr.clips.insert(idx, *original);
                Ok(())
            }
            Inverse::UndoRippleDelete { sequence, track, restored } => {
                let seq = project
                    .sequences
                    .iter_mut()
                    .find(|s| s.id == sequence)
                    .ok_or(EditError::UnknownSequence(sequence))?;
                let tr = seq
                    .track_mut(track)
                    .ok_or(EditError::UnknownTrack(track))?;
                tr.clips = restored;
                Ok(())
            }
            Inverse::UndoRippleMove { sequence, track, from, moved } => {
                let seq = project
                    .sequences
                    .iter_mut()
                    .find(|s| s.id == sequence)
                    .ok_or(EditError::UnknownSequence(sequence))?;
                let tr = seq
                    .track_mut(track)
                    .ok_or(EditError::UnknownTrack(track))?;
                if let Some(c) = tr.clips.iter_mut().find(|c| moved.iter().any(|(id, _)| *id == c.id)) {
                    c.timeline_start = from;
                }
                for (id, old) in moved {
                    if let Some(c) = tr.clips.iter_mut().find(|c| c.id == id) {
                        c.timeline_start = old;
                    }
                }
                Ok(())
            }
        }
    }
}

/// Find a clip in any track and set its timeline start. Returns an error only
/// if the clip is genuinely gone, which would be a bug in the caller rather
/// than a user error.
fn restore_start(project: &mut Project, clip: ClipId, from: Ticks) -> Result<(), EditError> {
    for seq in project.sequences.iter_mut() {
        for tr in seq.tracks.iter_mut() {
            if let Some(c) = tr.clips.iter_mut().find(|c| c.id == clip) {
                c.timeline_start = from;
                return Ok(());
            }
        }
    }
    Err(EditError::UnknownClip(clip))
}

/// Undo a trim. The head trim moved `timeline_start` by the same delta it
/// moved `source_in`, so undoing it means shifting the start back by the delta
/// between the current `source_in` and the original one. Computing that delta
/// against the *current* value is the whole trick: computing it against `from`
/// would double-count.
fn restore_edge(
    project: &mut Project,
    clip: ClipId,
    edge: TrimEdge,
    from: Ticks,
) -> Result<(), EditError> {
    for seq in project.sequences.iter_mut() {
        for tr in seq.tracks.iter_mut() {
            if let Some(c) = tr.clips.iter_mut().find(|c| c.id == clip) {
                match edge {
                    TrimEdge::In => {
                        let delta = c.source_in.raw() - from.raw();
                        c.timeline_start = Ticks(c.timeline_start.raw() - delta);
                        c.source_in = from;
                    }
                    TrimEdge::Out => c.source_out = from,
                }
                return Ok(());
            }
        }
    }
    Err(EditError::UnknownClip(clip))
}

/// A small description of what changed, sent to the UI after each op
/// (architecture rule 10).
///
/// The UI never receives a full snapshot except on open and after recovery.
/// A drag is a transient local op in the UI and becomes exactly one op on
/// release, so no IPC round trip happens during the drag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum Patch {
    ClipInserted { sequence: SequenceId, track: TrackId, index: usize, clip: Box<Clip> },
    ClipRemoved { sequence: SequenceId, track: TrackId, index: usize, clip_id: ClipId },
    ClipChanged { sequence: SequenceId, track: TrackId, clip_id: ClipId },
    TrackChanged { sequence: SequenceId, track: TrackId },
    SequenceChanged { sequence: SequenceId },
}

impl Patch {
    /// Derive the patch from the operation and its inverse.
    ///
    /// Pure, so the UI view of the project cannot drift from the model: there
    /// is exactly one place where a change is described, and it is a function.
    ///
    /// Matching on the op alone keeps this exhaustive: adding an [`EditOp`]
    /// variant is then a compile error here rather than a silently missing
    /// UI update.
    pub fn from(op: &EditOp, inverse: &Inverse) -> Patch {
        // The inverse is used only to recover the index a removal happened at.
        // Everything else is determined by the op.
        match op {
            EditOp::InsertClip { sequence, track, index, clip } => Patch::ClipInserted {
                sequence: *sequence,
                track: *track,
                index: *index,
                clip: clip.clone(),
            },
            EditOp::DeleteClip { sequence, track, clip } => {
                let index = match inverse {
                    Inverse::UndoDeleteClip { index, .. } => *index,
                    _ => 0,
                };
                Patch::ClipRemoved {
                    sequence: *sequence,
                    track: *track,
                    index,
                    clip_id: *clip,
                }
            }
            EditOp::MoveClip { sequence, track, clip, .. }
            | EditOp::TrimClip { sequence, track, clip, .. }
            | EditOp::RippleMove { sequence, track, clip, .. } => Patch::ClipChanged {
                sequence: *sequence,
                track: *track,
                clip_id: *clip,
            },
            EditOp::SplitClip { sequence, track, .. }
            | EditOp::RippleDelete { sequence, track, .. } => Patch::TrackChanged {
                sequence: *sequence,
                track: *track,
            },
        }
    }
}

impl Project {
    /// Apply one operation, returning how to undo it.
    pub fn apply(&mut self, op: EditOp) -> Result<Inverse, EditError> {
        self.apply_inner(op)
    }

    fn apply_inner(&mut self, op: EditOp) -> Result<Inverse, EditError> {
        match op {
            EditOp::InsertClip { sequence, track, index, clip } => {
                let clip_id = clip.id;
                let tr = self.track_mut_checked(sequence, track)?;
                if index < tr.clips.len() {
                    tr.clips.insert(index, *clip);
                } else {
                    tr.clips.push(*clip);
                }
                Ok(Inverse::UndoInsertClip { sequence, track, clip: clip_id })
            }

            EditOp::DeleteClip { sequence, track, clip } => {
                let tr = self.track_mut_checked(sequence, track)?;
                let index = tr
                    .clips
                    .iter()
                    .position(|c| c.id == clip)
                    .ok_or(EditError::UnknownClip(clip))?;
                let removed = tr.clips.remove(index);
                Ok(Inverse::UndoDeleteClip {
                    sequence,
                    track,
                    index,
                    clip: Box::new(removed),
                })
            }

            EditOp::MoveClip { sequence, track, clip, to } => {
                let tr = self.track_mut_checked(sequence, track)?;
                let c = tr
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip)
                    .ok_or(EditError::UnknownClip(clip))?;
                let from = c.timeline_start;
                c.timeline_start = to.saturating_non_negative();
                Ok(Inverse::UndoMoveClip { clip, from })
            }

            EditOp::TrimClip { sequence, track, clip, edge, to } => {
                let tr = self.track_mut_checked(sequence, track)?;
                let c = tr
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip)
                    .ok_or(EditError::UnknownClip(clip))?;
                let from = match edge {
                    TrimEdge::In => c.source_in,
                    TrimEdge::Out => c.source_out,
                };
                match edge {
                    TrimEdge::In => {
                        if to > c.source_out {
                            return Err(EditError::InvertedRange);
                        }
                        // Shift the timeline start by the same amount so the
                        // picture under the clip stays put while the head
                        // reveals more or less of the source.
                        let delta = to.raw() - c.source_in.raw();
                        c.source_in = to;
                        c.timeline_start = Ticks(c.timeline_start.raw() + delta);
                    }
                    TrimEdge::Out => {
                        if to < c.source_in {
                            return Err(EditError::InvertedRange);
                        }
                        c.source_out = to;
                    }
                }
                Ok(Inverse::UndoTrimClip { clip, edge, from })
            }

            EditOp::SplitClip { sequence, track, clip, at } => {
                let tr = self.track_mut_checked(sequence, track)?;
                let index = tr
                    .clips
                    .iter()
                    .position(|c| c.id == clip)
                    .ok_or(EditError::UnknownClip(clip))?;
                let original = tr.clips[index].clone();
                if at <= original.source_in || at >= original.source_out {
                    return Err(EditError::InvertedRange);
                }
                // The left half keeps the original id: it *is* the clip the
                // user was looking at, just shortened. Only the tail gets a new
                // id. This matters because link groups and the UI's selection
                // both refer to the clip by id across a split.
                let mut left = original.clone();
                left.source_out = at;
                let mut right = original.clone();
                right.id = ClipId::new_v4();
                right.source_in = at;
                right.timeline_start = Ticks(left.timeline_start.raw() + left.duration().raw());

                tr.clips[index] = left.clone();
                tr.clips.insert(index + 1, right.clone());
                Ok(Inverse::UndoSplitClip {
                    sequence,
                    track,
                    index,
                    original: Box::new(original),
                    halves: vec![left, right],
                })
            }

            EditOp::RippleDelete { sequence, track, range } => {
                let seq = self
                    .sequences
                    .iter_mut()
                    .find(|s| s.id == sequence)
                    .ok_or(EditError::UnknownSequence(sequence))?;
                let tr = seq
                    .track_mut(track)
                    .ok_or(EditError::UnknownTrack(track))?;
                if tr.locked {
                    return Err(EditError::TrackLocked);
                }
                // Undo stores the whole prior clip list. This is O(n) memory
                // per ripple delete, which is the honest cost of making undo
                // exact rather than approximate.
                let before = tr.clips.clone();
                let mut kept: Vec<Clip> = Vec::with_capacity(before.len());
                for c in before.iter() {
                    if !c.overlaps_range(range) {
                        kept.push(c.clone());
                        continue;
                    }
                    if c.timeline_start < range.start {
                        let mut head = c.clone();
                        head.source_out =
                            Ticks(head.source_in.raw() + (range.start.raw() - c.timeline_start.raw()));
                        kept.push(head);
                    }
                    if c.end() > range.end {
                        let mut tail = c.clone();
                        tail.source_in = Ticks(c.source_out.raw() - (c.end().raw() - range.end.raw()));
                        tail.timeline_start = range.start;
                        kept.push(tail);
                    }
                }
                tr.clips = kept;
                Ok(Inverse::UndoRippleDelete { sequence, track, restored: before })
            }

            EditOp::RippleMove { sequence, track, clip, to } => {
                let tr = self.track_mut_checked(sequence, track)?;
                let idx = tr
                    .clips
                    .iter()
                    .position(|c| c.id == clip)
                    .ok_or(EditError::UnknownClip(clip))?;
                let from = tr.clips[idx].timeline_start;
                let shift = to.raw() - from.raw();
                if shift == 0 {
                    return Ok(Inverse::NoOp);
                }
                let mut moved = Vec::new();
                for c in tr.clips.iter_mut() {
                    if c.timeline_start >= from {
                        let old = c.timeline_start;
                        c.timeline_start = Ticks(old.raw() + shift);
                        moved.push((c.id, old));
                    }
                }
                Ok(Inverse::UndoRippleMove { sequence, track, from, moved })
            }
        }
    }

    fn track_mut_checked(
        &mut self,
        sequence: SequenceId,
        track: TrackId,
    ) -> Result<&mut Track, EditError> {
        let seq = self
            .sequences
            .iter_mut()
            .find(|s| s.id == sequence)
            .ok_or(EditError::UnknownSequence(sequence))?;
        let tr = seq
            .track_mut(track)
            .ok_or(EditError::UnknownTrack(track))?;
        if tr.locked {
            return Err(EditError::TrackLocked);
        }
        Ok(tr)
    }
}

impl Clip {
    fn overlaps_range(&self, r: TimeRange) -> bool {
        self.timeline_start < r.end && r.start < self.end()
    }
}