//! Append-only history graph with patch-based undo/redo.
//!
//! Each [`HistoryNode`] is a single edit. Edits carry enough
//! information to invert themselves (`from`/`to` field pairs, or
//! `previous` snapshots for layer operations). Undo and redo are
//! sequences of forward/inverse application; no snapshot retention
//! required.
//!
//! ## v0 constraints
//!
//! - **Linear only.** Committing after a checkout-backward truncates
//!   any node that's not an ancestor of the new head — git-style
//!   detached-HEAD-then-commit. Branching (and full CRDT merge) is
//!   deferred to Feature Target 9.
//! - **Phrase pool is append-only.** Inverting an `AppendLayer` does
//!   not remove the phrase from `session.phrases` — only the layer
//!   pop is undone. The pool grows monotonically.
//! - **Layers are append-only.** v0 doesn't have a `RemoveLayer`
//!   edit; "remove from playback" is `SetLayerMute(.., to: true)`.
//!   Mix-down (a future user gesture) will consolidate layers, but
//!   that operation goes via a different edit shape.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ids::{NodeId, TrackId};
use crate::phrase::{Layer, Phrase};
use crate::session::{Session, TimeSignature};
use crate::track::{PlaybackMode, TrackColor};

/// A single recorded edit. Carries its own inversion data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Edit {
    /// Root sentinel — present once per history, at the root node.
    Genesis,

    SetBpm { from: f32, to: f32 },
    SetTimeSignature { from: TimeSignature, to: TimeSignature },
    SetBarsPerPhrase { from: u8, to: u8 },
    SetMasterClock { from: bool, to: bool },
    SetCountInBars { from: u8, to: u8 },

    RenameTrack {
        track_id: TrackId,
        from: String,
        to: String,
    },
    SetTrackColor {
        track_id: TrackId,
        from: TrackColor,
        to: TrackColor,
    },
    ArmTrack {
        track_id: TrackId,
        from: bool,
        to: bool,
    },
    MuteTrack {
        track_id: TrackId,
        from: bool,
        to: bool,
    },

    /// Append a new layer to a track. Apply inserts the phrase into
    /// the pool (if absent) and pushes the layer onto `Track.layers`.
    /// Invert pops the layer (layers are append-only, so the layer
    /// being inverted is always the last one). The phrase remains in
    /// the pool — monotonic-pool invariant.
    AppendLayer {
        track_id: TrackId,
        phrase: Phrase,
        layer: Layer,
    },
    /// Change a layer's gain. Apply replaces the gain; invert
    /// restores `from`.
    SetLayerGain {
        track_id: TrackId,
        layer_index: u16,
        from: f32,
        to: f32,
    },
    /// Change a layer's mute state. Apply sets `muted = to`; invert
    /// restores `from`.
    SetLayerMute {
        track_id: TrackId,
        layer_index: u16,
        from: bool,
        to: bool,
    },

    /// Change a track's playback mode (Sum vs SelectOne). This is the
    /// load-bearing edit for switching between the looper-pedal
    /// profile and the Deeler profile per-track.
    SetTrackPlaybackMode {
        track_id: TrackId,
        from: PlaybackMode,
        to: PlaybackMode,
    },
    /// Pick the currently-active layer in `SelectOne` mode (the
    /// Deeler variation-picking gesture). No-op on `Sum`-mode tracks.
    /// `from` and `to` are the active-layer indices before and after.
    SelectActiveLayer {
        track_id: TrackId,
        from: Option<u16>,
        to: Option<u16>,
    },
}

impl Edit {
    /// Apply this edit forward to the given session.
    pub fn apply(&self, session: &mut Session) {
        match self {
            Edit::Genesis => {}
            Edit::SetBpm { to, .. } => session.bpm = *to,
            Edit::SetTimeSignature { to, .. } => session.time_signature = *to,
            Edit::SetBarsPerPhrase { to, .. } => session.bars_per_phrase = *to,
            Edit::SetMasterClock { to, .. } => session.master_clock_enabled = *to,
            Edit::SetCountInBars { to, .. } => session.count_in_bars = *to,
            Edit::RenameTrack { track_id, to, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.name = to.clone();
                }
            }
            Edit::SetTrackColor { track_id, to, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.color = *to;
                }
            }
            Edit::ArmTrack { track_id, to, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.armed = *to;
                }
            }
            Edit::MuteTrack { track_id, to, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.muted = *to;
                }
            }
            Edit::AppendLayer {
                track_id,
                phrase,
                layer,
            } => {
                session.phrases.entry(phrase.id).or_insert_with(|| phrase.clone());
                if let Some(t) = session.track_mut(*track_id) {
                    t.layers.push(*layer);
                }
            }
            Edit::SetLayerGain {
                track_id,
                layer_index,
                to,
                ..
            } => {
                if let Some(t) = session.track_mut(*track_id) {
                    if let Some(layer) = t.layers.get_mut(*layer_index as usize) {
                        layer.gain = *to;
                    }
                }
            }
            Edit::SetLayerMute {
                track_id,
                layer_index,
                to,
                ..
            } => {
                if let Some(t) = session.track_mut(*track_id) {
                    if let Some(layer) = t.layers.get_mut(*layer_index as usize) {
                        layer.muted = *to;
                    }
                }
            }
            Edit::SetTrackPlaybackMode { track_id, to, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.playback_mode = *to;
                }
            }
            Edit::SelectActiveLayer { track_id, to, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    if let PlaybackMode::SelectOne { active } = &mut t.playback_mode {
                        *active = *to;
                    }
                    // No-op on Sum-mode tracks (documented).
                }
            }
        }
    }

    /// Invert this edit, restoring the session to its pre-apply state.
    pub fn invert(&self, session: &mut Session) {
        match self {
            Edit::Genesis => {}
            Edit::SetBpm { from, .. } => session.bpm = *from,
            Edit::SetTimeSignature { from, .. } => session.time_signature = *from,
            Edit::SetBarsPerPhrase { from, .. } => session.bars_per_phrase = *from,
            Edit::SetMasterClock { from, .. } => session.master_clock_enabled = *from,
            Edit::SetCountInBars { from, .. } => session.count_in_bars = *from,
            Edit::RenameTrack { track_id, from, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.name = from.clone();
                }
            }
            Edit::SetTrackColor { track_id, from, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.color = *from;
                }
            }
            Edit::ArmTrack { track_id, from, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.armed = *from;
                }
            }
            Edit::MuteTrack { track_id, from, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.muted = *from;
                }
            }
            Edit::AppendLayer { track_id, .. } => {
                // Layers are append-only; the layer being undone is
                // always the most recent one pushed onto this track.
                if let Some(t) = session.track_mut(*track_id) {
                    t.layers.pop();
                }
                // The phrase stays in the pool (monotonic-pool invariant).
            }
            Edit::SetLayerGain {
                track_id,
                layer_index,
                from,
                ..
            } => {
                if let Some(t) = session.track_mut(*track_id) {
                    if let Some(layer) = t.layers.get_mut(*layer_index as usize) {
                        layer.gain = *from;
                    }
                }
            }
            Edit::SetLayerMute {
                track_id,
                layer_index,
                from,
                ..
            } => {
                if let Some(t) = session.track_mut(*track_id) {
                    if let Some(layer) = t.layers.get_mut(*layer_index as usize) {
                        layer.muted = *from;
                    }
                }
            }
            Edit::SetTrackPlaybackMode { track_id, from, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    t.playback_mode = *from;
                }
            }
            Edit::SelectActiveLayer { track_id, from, .. } => {
                if let Some(t) = session.track_mut(*track_id) {
                    if let PlaybackMode::SelectOne { active } = &mut t.playback_mode {
                        *active = *from;
                    }
                }
            }
        }
    }
}

/// A node in the history graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryNode {
    pub id: NodeId,
    /// `None` only for the root (Genesis) node.
    pub parent: Option<NodeId>,
    pub edit: Edit,
    /// Milliseconds since Unix epoch when this edit was committed.
    /// 0 is acceptable in tests where wall-clock time doesn't matter.
    pub timestamp_ms: u64,
}

/// Errors specific to history operations.
#[derive(Debug, PartialEq, Eq)]
pub enum HistoryError {
    /// Requested checkout target is not reachable from current head
    /// in the linear v0 history.
    NotInLineage(NodeId),
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInLineage(id) => {
                write!(f, "history checkout target {id} is not in current lineage")
            }
        }
    }
}

impl std::error::Error for HistoryError {}

/// The history graph plus current head pointer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct History {
    pub root: NodeId,
    pub head: NodeId,
    /// All live nodes. `BTreeMap` for deterministic serialization order.
    pub nodes: BTreeMap<NodeId, HistoryNode>,
}

impl History {
    /// New history with a Genesis root node.
    pub fn new() -> Self {
        let root = NodeId::new();
        let mut nodes = BTreeMap::new();
        nodes.insert(
            root,
            HistoryNode {
                id: root,
                parent: None,
                edit: Edit::Genesis,
                timestamp_ms: 0,
            },
        );
        Self {
            root,
            head: root,
            nodes,
        }
    }

    /// Commit an edit. Applies it to the session, appends a new node
    /// pointing at the current head, and advances head. If the
    /// current head has descendants (because of a prior
    /// checkout-backward), those descendants are truncated first —
    /// v0 is linear, so a new commit after detached HEAD invalidates
    /// the future it strayed from.
    pub fn commit(&mut self, edit: Edit, session: &mut Session, timestamp_ms: u64) -> NodeId {
        self.truncate_descendants_of(self.head);
        edit.apply(session);
        let id = NodeId::new();
        self.nodes.insert(
            id,
            HistoryNode {
                id,
                parent: Some(self.head),
                edit,
                timestamp_ms,
            },
        );
        self.head = id;
        id
    }

    /// Move the head to a different node, applying or inverting edits
    /// along the way so that `session` ends up matching the state at
    /// `target`. Linear-only in v0.
    pub fn checkout(
        &mut self,
        target: NodeId,
        session: &mut Session,
    ) -> Result<(), HistoryError> {
        if self.head == target {
            return Ok(());
        }
        if !self.nodes.contains_key(&target) {
            return Err(HistoryError::NotInLineage(target));
        }

        let head_chain = self.ancestor_chain(self.head);
        let target_chain = self.ancestor_chain(target);

        // Case 1: target is an ancestor of head — walk back, invert each.
        if let Some(idx) = head_chain.iter().position(|&id| id == target) {
            for &id in &head_chain[..idx] {
                let node = &self.nodes[&id];
                node.edit.invert(session);
            }
            self.head = target;
            return Ok(());
        }

        // Case 2: head is an ancestor of target — walk forward, apply each.
        if let Some(idx) = target_chain.iter().position(|&id| id == self.head) {
            let forward: Vec<NodeId> = target_chain[..idx].iter().rev().copied().collect();
            for id in forward {
                let node = &self.nodes[&id];
                node.edit.apply(session);
            }
            self.head = target;
            return Ok(());
        }

        Err(HistoryError::NotInLineage(target))
    }

    /// Walk from `start` up via parent pointers to the root, inclusive.
    fn ancestor_chain(&self, start: NodeId) -> Vec<NodeId> {
        let mut chain = Vec::new();
        let mut cur = Some(start);
        while let Some(id) = cur {
            chain.push(id);
            cur = self.nodes.get(&id).and_then(|n| n.parent);
        }
        chain
    }

    /// Remove every node that is not an ancestor (inclusive) of `keep`.
    fn truncate_descendants_of(&mut self, keep: NodeId) {
        let keepset: BTreeSet<NodeId> = self.ancestor_chain(keep).into_iter().collect();
        self.nodes.retain(|id, _| keepset.contains(id));
    }

    /// Whether there's an edit to undo (head is past the root).
    pub fn can_undo(&self) -> bool {
        self.nodes
            .get(&self.head)
            .and_then(|n| n.parent)
            .is_some()
    }

    /// Whether there's an edit to redo (head has a child node).
    pub fn can_redo(&self) -> bool {
        self.nodes.values().any(|n| n.parent == Some(self.head))
    }

    /// Undo the last edit: move head to its parent, inverting the edit
    /// against `session`. Returns `false` (no-op) if already at the
    /// root.
    pub fn undo(&mut self, session: &mut Session) -> bool {
        let Some(parent) = self.nodes.get(&self.head).and_then(|n| n.parent) else {
            return false;
        };
        self.checkout(parent, session).is_ok()
    }

    /// Redo: move head forward to its child (if any), re-applying that
    /// edit. v0 history is linear, so there's at most one child.
    /// Returns `false` (no-op) if head has no child.
    pub fn redo(&mut self, session: &mut Session) -> bool {
        let Some(child) = self
            .nodes
            .values()
            .find(|n| n.parent == Some(self.head))
            .map(|n| n.id)
        else {
            return false;
        };
        self.checkout(child, session).is_ok()
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::MediaRef;

    #[test]
    fn new_history_has_only_root() {
        let h = History::new();
        assert_eq!(h.head, h.root);
        assert_eq!(h.nodes.len(), 1);
        assert!(matches!(h.nodes[&h.root].edit, Edit::Genesis));
        assert_eq!(h.nodes[&h.root].parent, None);
    }

    #[test]
    fn commit_advances_head_and_applies() {
        let mut s = Session::new_default();
        let mut h = History::new();
        let n1 = h.commit(
            Edit::SetBpm {
                from: s.bpm,
                to: 90.0,
            },
            &mut s,
            0,
        );
        assert_eq!(h.head, n1);
        assert_eq!(s.bpm, 90.0);
        assert_eq!(h.nodes.len(), 2);
    }

    #[test]
    fn undo_redo_round_trips() {
        let mut s = Session::new_default();
        let mut h = History::new();
        assert!(!h.can_undo() && !h.can_redo());

        h.commit(Edit::SetBpm { from: 120.0, to: 90.0 }, &mut s, 0);
        assert_eq!(s.bpm, 90.0);
        assert!(h.can_undo() && !h.can_redo());

        assert!(h.undo(&mut s));
        assert_eq!(s.bpm, 120.0);
        assert!(!h.can_undo() && h.can_redo());

        assert!(h.redo(&mut s));
        assert_eq!(s.bpm, 90.0);

        // Undo back to root, then a further undo is a no-op.
        assert!(h.undo(&mut s));
        assert_eq!(s.bpm, 120.0);
        assert!(!h.undo(&mut s)); // already at root
        assert!(h.can_redo()); // but the edit is still redoable
    }

    #[test]
    fn checkout_back_inverts() {
        let mut s = Session::new_default();
        let mut h = History::new();
        h.commit(
            Edit::SetBpm {
                from: 120.0,
                to: 90.0,
            },
            &mut s,
            0,
        );
        assert_eq!(s.bpm, 90.0);
        h.checkout(h.root, &mut s).unwrap();
        assert_eq!(s.bpm, 120.0);
        assert_eq!(h.head, h.root);
    }

    #[test]
    fn checkout_forward_applies() {
        let mut s = Session::new_default();
        let mut h = History::new();
        let n1 = h.commit(
            Edit::SetBpm {
                from: 120.0,
                to: 90.0,
            },
            &mut s,
            0,
        );
        h.checkout(h.root, &mut s).unwrap();
        assert_eq!(s.bpm, 120.0);
        h.checkout(n1, &mut s).unwrap();
        assert_eq!(s.bpm, 90.0);
    }

    #[test]
    fn commit_after_checkout_back_truncates_future() {
        let mut s = Session::new_default();
        let mut h = History::new();
        let _n1 = h.commit(
            Edit::SetBpm { from: 120.0, to: 90.0 },
            &mut s,
            0,
        );
        let n2 = h.commit(
            Edit::SetBpm { from: 90.0, to: 60.0 },
            &mut s,
            0,
        );
        assert_eq!(h.nodes.len(), 3);

        h.checkout(h.root, &mut s).unwrap();
        let _n3 = h.commit(
            Edit::SetBpm { from: 120.0, to: 100.0 },
            &mut s,
            0,
        );

        assert!(!h.nodes.contains_key(&n2));
        assert_eq!(s.bpm, 100.0);
    }

    #[test]
    fn append_layer_round_trip() {
        let mut s = Session::new_default();
        let mut h = History::new();
        let track_id = s.tracks[0].id;
        let phrase = Phrase::new(MediaRef([1; 32]), 4, 120.0, 1000);
        let phrase_id = phrase.id;
        let layer = Layer::new(phrase_id);

        let n1 = h.commit(
            Edit::AppendLayer {
                track_id,
                phrase,
                layer,
            },
            &mut s,
            0,
        );

        assert_eq!(s.tracks[0].layers.len(), 1);
        assert_eq!(s.tracks[0].layers[0].phrase_id, phrase_id);
        assert!(s.phrases.contains_key(&phrase_id));

        // Step back: layer popped, phrase stays in pool.
        h.checkout(h.root, &mut s).unwrap();
        assert!(s.tracks[0].layers.is_empty());
        assert!(
            s.phrases.contains_key(&phrase_id),
            "phrase should remain in the pool across undo"
        );

        // Step forward: layer pushed back.
        h.checkout(n1, &mut s).unwrap();
        assert_eq!(s.tracks[0].layers.len(), 1);
        assert_eq!(s.tracks[0].layers[0].phrase_id, phrase_id);
    }

    #[test]
    fn two_appends_then_mute_one() {
        let mut s = Session::new_default();
        let mut h = History::new();
        let track_id = s.tracks[0].id;

        let phrase_a = Phrase::new(MediaRef([1; 32]), 4, 120.0, 1000);
        let layer_a = Layer::new(phrase_a.id);
        h.commit(
            Edit::AppendLayer {
                track_id,
                phrase: phrase_a,
                layer: layer_a,
            },
            &mut s,
            0,
        );

        let phrase_b = Phrase::new(MediaRef([2; 32]), 4, 120.0, 2000);
        let layer_b = Layer::new(phrase_b.id);
        h.commit(
            Edit::AppendLayer {
                track_id,
                phrase: phrase_b,
                layer: layer_b,
            },
            &mut s,
            0,
        );

        assert_eq!(s.tracks[0].layers.len(), 2);

        // Mute layer 0 (the first one).
        h.commit(
            Edit::SetLayerMute {
                track_id,
                layer_index: 0,
                from: false,
                to: true,
            },
            &mut s,
            0,
        );
        assert!(s.tracks[0].layers[0].muted);
        assert!(!s.tracks[0].layers[1].muted);

        // Undo the mute.
        h.checkout(
            h.nodes
                .values()
                .find(|n| {
                    matches!(
                        n.edit,
                        Edit::AppendLayer { ref phrase, .. } if phrase.captured_at_ms == 2000
                    )
                })
                .unwrap()
                .id,
            &mut s,
        )
        .unwrap();
        assert!(!s.tracks[0].layers[0].muted);
    }

    #[test]
    fn set_layer_gain_round_trip() {
        let mut s = Session::new_default();
        let mut h = History::new();
        let track_id = s.tracks[0].id;

        let phrase = Phrase::new(MediaRef([1; 32]), 4, 120.0, 1000);
        let layer = Layer::new(phrase.id);
        h.commit(
            Edit::AppendLayer {
                track_id,
                phrase,
                layer,
            },
            &mut s,
            0,
        );

        let after_append = h.head;

        h.commit(
            Edit::SetLayerGain {
                track_id,
                layer_index: 0,
                from: 1.0,
                to: 0.5,
            },
            &mut s,
            0,
        );
        assert_eq!(s.tracks[0].layers[0].gain, 0.5);

        h.checkout(after_append, &mut s).unwrap();
        assert_eq!(s.tracks[0].layers[0].gain, 1.0);
    }

    #[test]
    fn set_track_playback_mode_round_trip() {
        let mut s = Session::new_default();
        let mut h = History::new();
        let track_id = s.tracks[0].id;
        assert_eq!(s.tracks[0].playback_mode, PlaybackMode::Sum);

        let after = h.commit(
            Edit::SetTrackPlaybackMode {
                track_id,
                from: PlaybackMode::Sum,
                to: PlaybackMode::SelectOne { active: None },
            },
            &mut s,
            0,
        );
        assert_eq!(
            s.tracks[0].playback_mode,
            PlaybackMode::SelectOne { active: None }
        );

        // Undo: back to Sum.
        h.checkout(h.root, &mut s).unwrap();
        assert_eq!(s.tracks[0].playback_mode, PlaybackMode::Sum);

        // Redo: back to SelectOne.
        h.checkout(after, &mut s).unwrap();
        assert_eq!(
            s.tracks[0].playback_mode,
            PlaybackMode::SelectOne { active: None }
        );
    }

    #[test]
    fn select_active_layer_round_trip() {
        let mut s = Session::new_default();
        let mut h = History::new();
        let track_id = s.tracks[0].id;

        // Switch the track into SelectOne mode first.
        h.commit(
            Edit::SetTrackPlaybackMode {
                track_id,
                from: PlaybackMode::Sum,
                to: PlaybackMode::SelectOne { active: None },
            },
            &mut s,
            0,
        );
        let before_select = h.head;

        let after_select = h.commit(
            Edit::SelectActiveLayer {
                track_id,
                from: None,
                to: Some(2),
            },
            &mut s,
            0,
        );
        assert_eq!(
            s.tracks[0].playback_mode,
            PlaybackMode::SelectOne { active: Some(2) }
        );

        // Undo: back to None.
        h.checkout(before_select, &mut s).unwrap();
        assert_eq!(
            s.tracks[0].playback_mode,
            PlaybackMode::SelectOne { active: None }
        );

        // Redo: back to Some(2).
        h.checkout(after_select, &mut s).unwrap();
        assert_eq!(
            s.tracks[0].playback_mode,
            PlaybackMode::SelectOne { active: Some(2) }
        );
    }

    #[test]
    fn select_active_layer_on_sum_track_is_noop() {
        // SelectActiveLayer applied to a Sum-mode track does nothing
        // (documented behavior); the track stays in Sum mode.
        let mut s = Session::new_default();
        let mut h = History::new();
        let track_id = s.tracks[0].id;

        h.commit(
            Edit::SelectActiveLayer {
                track_id,
                from: None,
                to: Some(2),
            },
            &mut s,
            0,
        );
        assert_eq!(s.tracks[0].playback_mode, PlaybackMode::Sum);
    }

    #[test]
    fn unknown_target_yields_not_in_lineage() {
        let mut s = Session::new_default();
        let mut h = History::new();
        h.commit(
            Edit::SetBpm { from: 120.0, to: 90.0 },
            &mut s,
            0,
        );
        let bogus = NodeId::new();
        assert_eq!(
            h.checkout(bogus, &mut s),
            Err(HistoryError::NotInLineage(bogus))
        );
    }
}
