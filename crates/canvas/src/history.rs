//! Per-user undo/redo (CA-08), built on Loro's [`UndoManager`].
//!
//! The manager only records operations authored by this document's peer, so
//! undo never reverts the partner's edits. When remote updates arrive between
//! a local edit and its undo, Loro transforms the inverse against them, which
//! keeps both replicas convergent: an undo is just another local edit that
//! syncs through [`Board::take_pending_update`] like any other.
//!
//! # Grouping policy
//!
//! Every committed local edit is one undo step by default (the merge interval
//! is 0: there is no time-based merging). Gestures that commit many times
//! wrap their edits in [`Board::begin_group`] / [`Board::end_group`] so they
//! undo as a single step:
//!
//! - **Drag / resize:** begin on pointer down, end on pointer up/cancel.
//! - **Text editing:** begin when the editing session starts (the text field
//!   gains focus) and end when it finishes (blur / Escape / Enter to commit).
//!   While the field is focused, Ctrl+Z is the browser's native text undo;
//!   once the session ends the whole session undoes as one step.
//!
//! A group stays one step even when the partner's updates arrive in the
//! middle of it (see [`History`] for how). Undo always restores the state the
//! user saw before the gesture for the keys the gesture wrote.
//!
//! The camera is view state outside the document, so it is never undoable.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use loro::{LoroDoc, LoroValue, OnPush, UndoItemMeta, UndoManager};

use crate::Board;

/// Maximum number of undo items kept per user. A group interrupted by remote
/// updates (see [`History`]) takes one item per uninterrupted stretch.
pub const MAX_UNDO_STEPS: usize = 100;

/// Undo/redo stacks for the local peer of a document.
///
/// Loro can merge a whole group into one stack item, but undoing an item
/// whose ops have remote ops interleaved among them keeps only the last
/// sub-span's inverse for each key (earlier inverses are transformed away by
/// the later local writes), so a drag would only undo to its last
/// pre-import position. Instead, an open group is restarted around every
/// import, giving one Loro item per uninterrupted stretch, and every item is
/// tagged with the group id in its metadata; undo/redo replay all consecutive
/// items sharing a tag as one step.
pub(crate) struct History {
    manager: UndoManager,
    /// Tag stamped on items pushed right now: the open group's id, the id of
    /// the group being replayed by undo/redo, or 0 for "no group".
    tag: Arc<AtomicI64>,
    /// Id of the open group, if any.
    group: Option<i64>,
    next_group: i64,
}

impl History {
    pub(crate) fn new(doc: &LoroDoc) -> Self {
        let mut manager = UndoManager::new(doc);
        manager.set_max_undo_steps(MAX_UNDO_STEPS);
        manager.set_merge_interval(0);
        let tag = Arc::new(AtomicI64::new(0));
        let stamp = Arc::clone(&tag);
        let on_push: OnPush = Box::new(move |_, _, _| UndoItemMeta {
            value: match stamp.load(Ordering::Relaxed) {
                0 => LoroValue::Null,
                id => LoroValue::I64(id),
            },
            cursors: Vec::new(),
        });
        manager.set_on_push(Some(on_push));
        Self {
            manager,
            tag,
            group: None,
            next_group: 1,
        }
    }

    fn begin_group(&mut self) {
        self.end_group();
        let id = self.next_group;
        self.next_group += 1;
        self.open(id);
    }

    fn end_group(&mut self) {
        self.manager.group_end();
        self.group = None;
        self.tag.store(0, Ordering::Relaxed);
    }

    fn open(&mut self, id: i64) {
        // Only fails if the manager lost track of the local counter (after a
        // checkout, which the board never does); edits then stay ungrouped.
        if self.manager.group_start().is_ok() {
            self.group = Some(id);
            self.tag.store(id, Ordering::Relaxed);
        }
    }

    /// Pauses the open group before an import; pass the result to
    /// [`Self::after_import`] so no Loro item spans remote ops.
    pub(crate) fn before_import(&mut self) -> Option<i64> {
        let group = self.group;
        if group.is_some() {
            self.end_group();
        }
        group
    }

    pub(crate) fn after_import(&mut self, group: Option<i64>) {
        if let Some(id) = group {
            self.open(id);
        }
    }

    /// Replays one logical step from the undo (or redo) stack.
    fn replay(&mut self, redo: bool) -> bool {
        self.end_group();
        let top = |m: &UndoManager| {
            if redo {
                m.top_redo_value()
            } else {
                m.top_undo_value()
            }
        };
        let Some(first) = top(&self.manager) else {
            return false;
        };
        let grouped = matches!(first, LoroValue::I64(_));
        if let LoroValue::I64(id) = first {
            // Items pushed onto the opposite stack keep the group's tag.
            self.tag.store(id, Ordering::Relaxed);
        }
        let mut done = false;
        loop {
            // Undo only fails for spans missing from the oplog, which cannot
            // happen for the local peer's own history; treat it as a no-op.
            let step = if redo {
                self.manager.redo()
            } else {
                self.manager.undo()
            };
            done |= step.unwrap_or(false);
            if !grouped || top(&self.manager).as_ref() != Some(&first) {
                break;
            }
        }
        self.tag.store(0, Ordering::Relaxed);
        done
    }
}

impl Board {
    /// Reverts this user's most recent undo step. Returns `false` if there
    /// was nothing to undo. Closes any open group first.
    pub fn undo(&mut self) -> bool {
        self.history.replay(false)
    }

    /// Reapplies the most recently undone step. Returns `false` if there was
    /// nothing to redo. Closes any open group first.
    pub fn redo(&mut self) -> bool {
        self.history.replay(true)
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.history.manager.can_undo()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.history.manager.can_redo()
    }

    /// Starts a group: every local edit until [`Self::end_group`] becomes a
    /// single undo step. Calling it while a group is open closes that group
    /// and starts a new one, so a missed `end_group` cannot wedge history.
    pub fn begin_group(&mut self) {
        self.history.begin_group();
    }

    /// Closes the open group, if any. Safe to call without one.
    pub fn end_group(&mut self) {
        self.history.end_group();
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::{Board, Rect};

    fn sync(from: &mut Board, to: &mut Board) {
        if let Some(update) = from.take_pending_update().unwrap() {
            to.import(&update).unwrap();
        }
    }

    fn sync_both(a: &mut Board, b: &mut Board) {
        sync(a, b);
        sync(b, a);
    }

    fn pos(board: &Board, id: &str) -> Option<(f64, f64)> {
        board
            .rects()
            .into_iter()
            .find(|r| r.id == id)
            .map(|r| (r.x, r.y))
    }

    fn pair() -> (Board, Board) {
        let a = Board::new();
        a.set_peer_id(1);
        let b = Board::new();
        b.set_peer_id(2);
        (a, b)
    }

    #[test]
    fn undo_redo_create() {
        let mut board = Board::new();
        assert!(!board.can_undo());
        board.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        assert!(board.can_undo());
        assert!(board.undo());
        assert_eq!(board.rects(), Vec::<Rect>::new());
        assert!(!board.undo());
        assert!(board.redo());
        assert_eq!(pos(&board, "a"), Some((0.0, 0.0)));
        assert!(!board.redo());
    }

    #[test]
    fn undo_redo_move() {
        let mut board = Board::new();
        board.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        board.move_shape("a", 5.0, 6.0).unwrap();
        assert!(board.undo());
        assert_eq!(pos(&board, "a"), Some((0.0, 0.0)));
        assert!(board.redo());
        assert_eq!(pos(&board, "a"), Some((5.0, 6.0)));
    }

    #[test]
    fn undo_redo_delete() {
        let mut board = Board::new();
        board.add_rect("a", 1.0, 2.0, 3.0, 4.0);
        board.remove_shape("a");
        assert_eq!(board.rects(), Vec::<Rect>::new());
        assert!(board.undo());
        assert_eq!(
            board.rects(),
            vec![Rect {
                id: "a".into(),
                x: 1.0,
                y: 2.0,
                w: 3.0,
                h: 4.0
            }]
        );
        assert!(board.redo());
        assert_eq!(board.rects(), Vec::<Rect>::new());
        // The restored shape is editable again after a second undo.
        assert!(board.undo());
        board.move_shape("a", 9.0, 9.0).unwrap();
        assert_eq!(pos(&board, "a"), Some((9.0, 9.0)));
    }

    #[test]
    fn new_local_edit_clears_redo() {
        let mut board = Board::new();
        board.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        board.move_shape("a", 5.0, 5.0).unwrap();
        assert!(board.undo());
        assert!(board.can_redo());
        board.move_shape("a", 7.0, 7.0).unwrap();
        assert!(!board.can_redo());
        assert!(!board.redo());
        assert_eq!(pos(&board, "a"), Some((7.0, 7.0)));
    }

    #[test]
    fn drag_group_is_one_step() {
        let mut board = Board::new();
        board.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        board.begin_group();
        for i in 1..=20 {
            let v = f64::from(i);
            board.move_shape("a", v, v * 2.0).unwrap();
        }
        board.end_group();
        assert_eq!(pos(&board, "a"), Some((20.0, 40.0)));
        assert!(board.undo());
        assert_eq!(pos(&board, "a"), Some((0.0, 0.0)));
        assert!(board.redo());
        assert_eq!(pos(&board, "a"), Some((20.0, 40.0)));
        // The creation is still a separate, older step.
        assert!(board.undo());
        assert!(board.undo());
        assert_eq!(board.rects(), Vec::<Rect>::new());
    }

    #[test]
    fn begin_group_is_reentrant() {
        let mut board = Board::new();
        board.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        board.begin_group();
        board.move_shape("a", 1.0, 1.0).unwrap();
        // A second begin without end closes the first group.
        board.begin_group();
        board.move_shape("a", 2.0, 2.0).unwrap();
        board.end_group();
        board.end_group();
        assert!(board.undo());
        assert_eq!(pos(&board, "a"), Some((1.0, 1.0)));
        assert!(board.undo());
        assert_eq!(pos(&board, "a"), Some((0.0, 0.0)));
    }

    #[test]
    fn group_survives_unrelated_remote_edit() {
        let (mut a, mut b) = pair();
        a.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        sync_both(&mut a, &mut b);
        a.begin_group();
        a.move_shape("a", 1.0, 1.0).unwrap();
        b.add_rect("b", 50.0, 50.0, 10.0, 10.0);
        sync_both(&mut a, &mut b);
        a.move_shape("a", 2.0, 2.0).unwrap();
        b.move_shape("b", 60.0, 60.0).unwrap();
        sync_both(&mut a, &mut b);
        a.move_shape("a", 3.0, 3.0).unwrap();
        a.end_group();
        assert!(a.undo());
        assert_eq!(pos(&a, "a"), Some((0.0, 0.0)));
        assert_eq!(pos(&a, "b"), Some((60.0, 60.0)));
        assert!(a.redo());
        assert_eq!(pos(&a, "a"), Some((3.0, 3.0)));
        assert!(a.undo());
        sync_both(&mut a, &mut b);
        assert_eq!(pos(&b, "a"), Some((0.0, 0.0)));
        assert_eq!(a.rects(), b.rects());
        // The creation is the next step down.
        assert!(a.undo());
        assert_eq!(pos(&a, "a"), None);
    }

    #[test]
    fn group_with_conflicting_remote_edit_converges() {
        let (mut a, mut b) = pair();
        a.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        sync_both(&mut a, &mut b);
        a.begin_group();
        a.move_shape("a", 1.0, 1.0).unwrap();
        // The partner moves the very shape being dragged.
        b.move_shape("a", 99.0, 99.0).unwrap();
        sync_both(&mut a, &mut b);
        a.move_shape("a", 2.0, 2.0).unwrap();
        a.end_group();
        sync_both(&mut a, &mut b);
        assert!(a.undo());
        // The whole drag is one step: the next undo removes the shape.
        assert!(a.undo());
        assert_eq!(pos(&a, "a"), None);
        sync_both(&mut a, &mut b);
        assert_eq!(a.rects(), b.rects());
    }

    #[test]
    fn history_is_capped() {
        let mut board = Board::new();
        board.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        for i in 1..=150 {
            board.move_shape("a", f64::from(i), 0.0).unwrap();
        }
        let mut undone = 0;
        while board.undo() {
            undone += 1;
        }
        assert_eq!(undone, super::MAX_UNDO_STEPS);
        // The oldest steps (creation and the first 50 moves) fell off.
        assert_eq!(pos(&board, "a"), Some((50.0, 0.0)));
    }

    #[test]
    fn undo_never_reverts_remote_edits() {
        let (mut a, mut b) = pair();
        a.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        sync_both(&mut a, &mut b);

        // Interleaved: A moves its shape, B creates and moves another one,
        // then A moves again.
        a.move_shape("a", 10.0, 10.0).unwrap();
        b.add_rect("b", 100.0, 100.0, 10.0, 10.0);
        sync_both(&mut a, &mut b);
        b.move_shape("b", 200.0, 200.0).unwrap();
        a.move_shape("a", 20.0, 20.0).unwrap();
        sync_both(&mut a, &mut b);

        // A undoes everything it did; B's shape stays exactly where B left it.
        assert!(a.undo());
        assert_eq!(pos(&a, "a"), Some((10.0, 10.0)));
        assert!(a.undo());
        assert!(a.undo());
        assert!(!a.undo());
        assert_eq!(pos(&a, "a"), None);
        assert_eq!(pos(&a, "b"), Some((200.0, 200.0)));

        sync_both(&mut a, &mut b);
        assert_eq!(a.rects(), b.rects());

        // B never did anything A could take from it, and B's own history is
        // untouched by A's undos.
        assert!(b.undo());
        assert_eq!(pos(&b, "b"), Some((100.0, 100.0)));
        sync_both(&mut a, &mut b);
        assert_eq!(a.rects(), b.rects());

        // Redo on A brings A's shape back without touching B's.
        assert!(a.redo());
        sync_both(&mut a, &mut b);
        assert_eq!(pos(&b, "a"), Some((0.0, 0.0)));
        assert_eq!(pos(&b, "b"), Some((100.0, 100.0)));
    }

    #[test]
    fn undo_reaches_partner_through_pending_update() {
        let (mut a, mut b) = pair();
        a.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        a.move_shape("a", 5.0, 5.0).unwrap();
        sync_both(&mut a, &mut b);
        assert!(a.undo());
        assert!(a.take_pending_update().unwrap().is_some_and(|u| {
            b.import(&u).unwrap();
            true
        }));
        assert_eq!(pos(&b, "a"), Some((0.0, 0.0)));
    }

    #[derive(Debug, Clone)]
    enum Op {
        Add(u8),
        Move(u8, i16, i16),
        Remove(u8),
        Undo,
        Redo,
        BeginGroup,
        EndGroup,
        Sync,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            2 => (0u8..4).prop_map(Op::Add),
            4 => (0u8..4, -50i16..50, -50i16..50).prop_map(|(s, x, y)| Op::Move(s, x, y)),
            1 => (0u8..4).prop_map(Op::Remove),
            3 => Just(Op::Undo),
            2 => Just(Op::Redo),
            1 => Just(Op::BeginGroup),
            1 => Just(Op::EndGroup),
            2 => Just(Op::Sync),
        ]
    }

    fn apply(board: &mut Board, peer: char, op: &Op) {
        match *op {
            Op::Add(s) => board.add_rect(&format!("{peer}{s}"), 0.0, 0.0, 10.0, 10.0),
            // Either peer may move or delete any shape, including the other's.
            Op::Move(s, x, y) => {
                let id = format!("{}{s}", if x % 2 == 0 { 'a' } else { 'b' });
                let _ = board.move_shape(&id, f64::from(x), f64::from(y));
            }
            Op::Remove(s) => board.remove_shape(&format!("{peer}{s}")),
            Op::Undo => {
                board.undo();
            }
            Op::Redo => {
                board.redo();
            }
            Op::BeginGroup => board.begin_group(),
            Op::EndGroup => board.end_group(),
            Op::Sync => {}
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Random interleaved local/remote edits mixed with undo/redo on both
        /// sides always converge once every update has been exchanged.
        #[test]
        fn interleaved_undo_redo_converges(
            ops in prop::collection::vec((any::<bool>(), op()), 1..60)
        ) {
            let (mut a, mut b) = pair();
            for (on_a, op) in &ops {
                if matches!(op, Op::Sync) {
                    sync_both(&mut a, &mut b);
                } else if *on_a {
                    apply(&mut a, 'a', op);
                } else {
                    apply(&mut b, 'b', op);
                }
            }
            sync_both(&mut a, &mut b);
            prop_assert_eq!(a.rects(), b.rects());
        }
    }
}
