//! The model-level guarantee that everything else is built on.
//!
//! # The property
//!
//! A random sequence of valid ops, followed by undoing every one of them in
//! reverse, must return the project to a **byte-identical** serialized state.
//!
//! Not "visually equivalent", not "the same clips in the same places". The
//! same bytes. That is stricter than it sounds and it is the point: an undo
//! stack that only approximately restores state produces projects that
//! silently drift after a hundred edits, and the drift is invisible until
//! someone reopens the file weeks later.

use quillie_core::project::{
    Clip, EditOp, Inverse, Project, SequenceId, TimeRange, Track, TrackId, TrackKind, TrimEdge,
};
use quillie_core::time::{FrameRate, Ticks};
use proptest::prelude::*;

/// A project with one video and one audio track, ready for editing.
fn fixture() -> (Project, SequenceId, TrackId, TrackId) {
    let mut p = Project::new("test", FrameRate::NTSC_30, (1920, 1080));
    let seq_id = p.primary_sequence().id;
    let video = Track::new(TrackKind::Video, "V1");
    let audio = Track::new(TrackKind::Audio, "A1");
    let vid = video.id;
    let aud = audio.id;
    p.primary_sequence_mut().tracks.push(video);
    p.primary_sequence_mut().tracks.push(audio);
    (p, seq_id, vid, aud)
}

/// A deterministic clip: the id is supplied so generated ops can refer to a
/// clip that is known to exist.
fn clip_at(id: uuid::Uuid, start_s: i64, len_s: i64) -> Clip {
    let mut c = Clip::new(
        None,
        Ticks::from_seconds(start_s),
        Ticks::from_seconds(0),
        Ticks::from_seconds(len_s),
    );
    c.id = id;
    c
}

/// The set of ops a property run may apply, resolved against the live project
/// so generated ops are always applicable.
fn candidate_ops(p: &Project, seq: SequenceId, video: TrackId, audio: TrackId) -> Vec<EditOp> {
    let mut ops = Vec::new();
    let track = p.primary_sequence();
    for t in [&video, &audio] {
        for c in &track.track(*t).unwrap().clips {
            let len = c.source_duration().raw() / 2;
            let mid = Ticks(c.source_in.raw() + len);
            ops.push(EditOp::MoveClip {
                sequence: seq,
                track: *t,
                clip: c.id,
                to: Ticks(c.timeline_start.raw() + 1000),
            });
            if mid > c.source_in && mid < c.source_out {
                ops.push(EditOp::TrimClip {
                    sequence: seq,
                    track: *t,
                    clip: c.id,
                    edge: TrimEdge::Out,
                    to: mid,
                });
                ops.push(EditOp::SplitClip {
                    sequence: seq,
                    track: *t,
                    clip: c.id,
                    at: mid,
                });
            }
            ops.push(EditOp::RippleDelete {
                sequence: seq,
                track: *t,
                range: TimeRange::new(c.timeline_start, Ticks(c.timeline_start.raw() + 500)),
            });
            ops.push(EditOp::RippleMove {
                sequence: seq,
                track: *t,
                clip: c.id,
                to: Ticks(c.timeline_start.raw() + 2000),
            });
        }
        ops.push(EditOp::InsertClip {
            sequence: seq,
            track: *t,
            index: 0,
            clip: Box::new(Clip::new(
                None,
                Ticks::from_seconds(1),
                Ticks::from_seconds(0),
                Ticks::from_seconds(3),
            )),
        });
    }
    ops
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// The central guarantee: ops then undos is the identity.
    #[test]
    fn random_ops_then_undo_restores_identical_state(
        steps in prop::collection::vec(0usize..6, 0..12)
    ) {
        let (mut p, seq, video, audio) = fixture();
        // The baseline is the empty project. The seed inserts are applied and
        // pushed onto the same inverse stack as everything after them, so
        // undoing the whole stack must land back here.
        let baseline = serde_json::to_string(&p).unwrap();

        // Seed with a couple of clips so there is something to edit.
        let seed_ids: Vec<uuid::Uuid> = (0..3).map(|_| uuid::Uuid::new_v4()).collect();
        let mut inverses: Vec<Inverse> = Vec::new();
        for (i, id) in seed_ids.iter().enumerate() {
            inverses.push(
                p.apply(EditOp::InsertClip {
                    sequence: seq,
                    track: video,
                    index: i,
                    clip: Box::new(clip_at(*id, i as i64 * 10, 8)),
                })
                .unwrap(),
            );
        }

        for step in steps {
            let ops = candidate_ops(&p, seq, video, audio);
            if ops.is_empty() { continue; }
            let op = ops[step % ops.len()].clone();
            if let Ok(inv) = p.apply(op) {
                inverses.push(inv);
            }
        }

        // Undo in reverse.
        while let Some(inv) = inverses.pop() {
            inv.undo(&mut p).unwrap();
        }

        let after = serde_json::to_string(&p).unwrap();
        prop_assert_eq!(&baseline, &after, "undo did not restore the exact state");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quillie_core::project::ClipId;

    #[test]
    fn undo_restores_a_deleted_clip_at_the_same_index() {
        let (mut p, seq, video, _audio) = fixture();
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        p.apply(EditOp::InsertClip {
            sequence: seq,
            track: video,
            index: 0,
            clip: Box::new(clip_at(a, 0, 5)),
        }).unwrap();
        p.apply(EditOp::InsertClip {
            sequence: seq,
            track: video,
            index: 1,
            clip: Box::new(clip_at(b, 10, 5)),
        }).unwrap();

        let before = serde_json::to_string(&p).unwrap();
        let inv = p.apply(EditOp::DeleteClip { sequence: seq, track: video, clip: b }).unwrap();
        assert_eq!(p.primary_sequence().track(video).unwrap().clips.len(), 1);
        inv.undo(&mut p).unwrap();
        assert_eq!(serde_json::to_string(&p).unwrap(), before);
    }

    #[test]
    fn trimming_the_head_moves_the_clip_start_with_it() {
        // The property: revealing more of the source must not slide the picture.
        let (mut p, seq, video, _audio) = fixture();
        let id = uuid::Uuid::new_v4();
        p.apply(EditOp::InsertClip {
            sequence: seq,
            track: video,
            index: 0,
            clip: Box::new(clip_at(id, 100, 10)),
        }).unwrap();
        p.apply(EditOp::TrimClip {
            sequence: seq,
            track: video,
            clip: id,
            edge: TrimEdge::In,
            to: Ticks::from_seconds(3),
        }).unwrap();
        let c = &p.primary_sequence().track(video).unwrap().clips[0];
        assert_eq!(c.source_in, Ticks::from_seconds(3));
        // start moved forward by the same 3 seconds
        assert_eq!(c.timeline_start, Ticks::from_seconds(103));
    }

    #[test]
    fn split_produces_two_contiguous_halves() {
        let (mut p, seq, video, _audio) = fixture();
        let id = uuid::Uuid::new_v4();
        p.apply(EditOp::InsertClip {
            sequence: seq,
            track: video,
            index: 0,
            clip: Box::new(clip_at(id, 0, 10)),
        }).unwrap();
        p.apply(EditOp::SplitClip {
            sequence: seq,
            track: video,
            clip: id,
            at: Ticks::from_seconds(4),
        }).unwrap();
        let clips = &p.primary_sequence().track(video).unwrap().clips;
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].source_out, Ticks::from_seconds(4));
        assert_eq!(clips[1].source_in, Ticks::from_seconds(4));
        assert_eq!(clips[1].timeline_start, clips[0].end());
        // no source material lost or duplicated
        let total: i64 = clips.iter().map(|c| c.source_duration().raw()).sum();
        assert_eq!(total, Ticks::from_seconds(10).raw());
    }

    #[test]
    fn undoing_a_split_restores_the_original_exactly() {
        let (mut p, seq, video, _audio) = fixture();
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        let c = uuid::Uuid::new_v4();
        p.apply(EditOp::InsertClip { sequence: seq, track: video, index: 0,
            clip: Box::new(clip_at(a, 0, 10)) }).unwrap();
        p.apply(EditOp::InsertClip { sequence: seq, track: video, index: 1,
            clip: Box::new(clip_at(b, 20, 10)) }).unwrap();
        p.apply(EditOp::InsertClip { sequence: seq, track: video, index: 2,
            clip: Box::new(clip_at(c, 40, 10)) }).unwrap();

        let before = serde_json::to_string(&p).unwrap();
        let inv = p.apply(EditOp::SplitClip {
            sequence: seq, track: video, clip: a, at: Ticks::from_seconds(5),
        }).unwrap();
        assert_eq!(p.primary_sequence().track(video).unwrap().clips.len(), 4);
        inv.undo(&mut p).unwrap();
        assert_eq!(p.primary_sequence().track(video).unwrap().clips.len(), 3);
        let after = serde_json::to_string(&p).unwrap();
        assert_eq!(before, after, "undo of split must restore byte-identical state");
    }

    #[test]
    fn undoing_a_ripple_delete_restores_the_track_exactly() {
        let (mut p, seq, video, _audio) = fixture();
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        p.apply(EditOp::InsertClip { sequence: seq, track: video, index: 0,
            clip: Box::new(clip_at(a, 0, 10)) }).unwrap();
        p.apply(EditOp::InsertClip { sequence: seq, track: video, index: 1,
            clip: Box::new(clip_at(b, 20, 10)) }).unwrap();
        let before = serde_json::to_string(&p).unwrap();
        let inv = p.apply(EditOp::RippleDelete {
            sequence: seq, track: video,
            range: TimeRange::new(Ticks::from_seconds(2), Ticks::from_seconds(5)),
        }).unwrap();
        inv.undo(&mut p).unwrap();
        assert_eq!(before, serde_json::to_string(&p).unwrap());
    }

    #[test]
    fn undoing_a_move_restores_the_start_time() {
        let (mut p, seq, video, _audio) = fixture();
        let a = uuid::Uuid::new_v4();
        p.apply(EditOp::InsertClip { sequence: seq, track: video, index: 0,
            clip: Box::new(clip_at(a, 5, 10)) }).unwrap();
        let before = serde_json::to_string(&p).unwrap();
        let inv = p.apply(EditOp::MoveClip {
            sequence: seq, track: video, clip: a, to: Ticks::from_seconds(50),
        }).unwrap();
        assert_eq!(p.primary_sequence().track(video).unwrap().clips[0].timeline_start,
                   Ticks::from_seconds(50));
        inv.undo(&mut p).unwrap();
        assert_eq!(before, serde_json::to_string(&p).unwrap());
    }

    #[test]
    fn undoing_a_trim_restores_both_edges() {
        let (mut p, seq, video, _audio) = fixture();
        let a = uuid::Uuid::new_v4();
        p.apply(EditOp::InsertClip { sequence: seq, track: video, index: 0,
            clip: Box::new(clip_at(a, 0, 10)) }).unwrap();
        let before = serde_json::to_string(&p).unwrap();
        let inv = p.apply(EditOp::TrimClip {
            sequence: seq, track: video, clip: a,
            edge: TrimEdge::In, to: Ticks::from_seconds(3),
        }).unwrap();
        inv.undo(&mut p).unwrap();
        assert_eq!(before, serde_json::to_string(&p).unwrap());
    }

    #[test]
    fn undoing_a_ripple_move_restores_every_shifted_clip() {
        let (mut p, seq, video, _audio) = fixture();
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        p.apply(EditOp::InsertClip { sequence: seq, track: video, index: 0,
            clip: Box::new(clip_at(a, 0, 10)) }).unwrap();
        p.apply(EditOp::InsertClip { sequence: seq, track: video, index: 1,
            clip: Box::new(clip_at(b, 20, 10)) }).unwrap();
        let before = serde_json::to_string(&p).unwrap();
        let inv = p.apply(EditOp::RippleMove {
            sequence: seq, track: video, clip: b, to: Ticks::from_seconds(60),
        }).unwrap();
        inv.undo(&mut p).unwrap();
        assert_eq!(before, serde_json::to_string(&p).unwrap());
    }

    #[test]
    fn split_then_ripple_move_undoes_cleanly() {
        let (mut p, seq, video, audio) = fixture();
        let a = uuid::Uuid::new_v4();
        for (i, id) in [a, uuid::Uuid::new_v4(), uuid::Uuid::new_v4()].iter().enumerate() {
            p.apply(EditOp::InsertClip { sequence: seq, track: video, index: i,
                clip: Box::new(clip_at(*id, i as i64 * 20, 10)) }).unwrap();
        }
        let baseline = serde_json::to_string(&p).unwrap();

        let ops: Vec<EditOp> = vec![
            EditOp::SplitClip { sequence: seq, track: video, clip: a, at: Ticks::from_seconds(5) },
            EditOp::RippleMove { sequence: seq, track: video, clip: a, to: Ticks::from_seconds(60) },
        ];
        let _ = audio;
        let mut inverses = Vec::new();
        for op in ops {
            inverses.push(p.apply(op).unwrap());
        }
        while let Some(i) = inverses.pop() {
            i.undo(&mut p).unwrap();
        }
        assert_eq!(baseline, serde_json::to_string(&p).unwrap());
    }

    #[test]
    fn an_unknown_clip_is_an_error_not_a_panic() {
        let (mut p, seq, video, _audio) = fixture();
        let missing = ClipId::new_v4();
        let err = p.apply(EditOp::MoveClip {
            sequence: seq,
            track: video,
            clip: missing,
            to: Ticks::from_seconds(1),
        });
        assert!(err.is_err());
    }

    #[test]
    fn a_locked_track_refuses_edits() {
        let (mut p, seq, video, _audio) = fixture();
        p.primary_sequence_mut().track_mut(video).unwrap().locked = true;
        let err = p.apply(EditOp::InsertClip {
            sequence: seq,
            track: video,
            index: 0,
            clip: Box::new(Clip::new(None, Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(1))),
        });
        assert!(err.is_err());
    }

    #[test]
    fn trim_past_the_other_edge_is_refused() {
        let (mut p, seq, video, _audio) = fixture();
        let id = uuid::Uuid::new_v4();
        p.apply(EditOp::InsertClip {
            sequence: seq,
            track: video,
            index: 0,
            clip: Box::new(clip_at(id, 0, 5)),
        }).unwrap();
        // source_in is 0, so trimming the head to 10s would invert the clip.
        let err = p.apply(EditOp::TrimClip {
            sequence: seq,
            track: video,
            clip: id,
            edge: TrimEdge::In,
            to: Ticks::from_seconds(10),
        });
        assert!(err.is_err());
    }
}