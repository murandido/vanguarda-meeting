//! QG-04: replicas that receive the same updates, in any order, end up with
//! the same board — including post-it text edited concurrently.

use canvas::{Board, Color, STICKERS};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    AddPostit(u8, f64, f64),
    AddSticker(u8, usize, f64, f64),
    Move(u8, f64, f64),
    Resize(u8, f64, f64),
    Recolor(u8, usize),
    Transform(u8, f64, f64),
    Remove(u8),
    /// Exchanges pending updates between the two replicas.
    Sync,
}

fn op() -> impl Strategy<Value = Op> {
    let id = 0u8..4;
    let coord = -1000.0..1000.0f64;
    prop_oneof![
        (id.clone(), coord.clone(), coord.clone()).prop_map(|(i, x, y)| Op::AddPostit(i, x, y)),
        (id.clone(), 0..STICKERS.len(), coord.clone(), coord.clone())
            .prop_map(|(i, e, x, y)| Op::AddSticker(i, e, x, y)),
        (id.clone(), coord.clone(), coord).prop_map(|(i, x, y)| Op::Move(i, x, y)),
        (id.clone(), 0.0..3000.0f64, 0.0..3000.0f64).prop_map(|(i, w, h)| Op::Resize(i, w, h)),
        (id.clone(), 0..Color::ALL.len()).prop_map(|(i, c)| Op::Recolor(i, c)),
        (id.clone(), 0.0..10.0f64, -10.0..10.0f64).prop_map(|(i, s, r)| Op::Transform(i, s, r)),
        id.prop_map(Op::Remove),
        Just(Op::Sync),
    ]
}

fn apply(board: &Board, op: &Op) {
    // Shape ids are shared between kinds on purpose: editing something the
    // other replica deleted or replaced with another kind is allowed, and
    // simply fails locally.
    let id = |i: u8| format!("s{i}");
    match *op {
        Op::AddPostit(i, x, y) => board
            .add_postit(&id(i), x, y, 120.0, 80.0, Color::Yellow)
            .unwrap(),
        Op::AddSticker(i, e, x, y) => board.add_sticker(&id(i), STICKERS[e], x, y).unwrap(),
        Op::Move(i, x, y) => {
            let _ = board.move_shape(&id(i), x, y);
        }
        Op::Resize(i, w, h) => {
            let _ = board.resize_postit(&id(i), w, h);
        }
        Op::Recolor(i, c) => {
            let _ = board.set_color(&id(i), Color::ALL[c]);
        }
        Op::Transform(i, s, r) => {
            let _ = board.transform_sticker(&id(i), s, r);
        }
        Op::Remove(i) => board.remove_shape(&id(i)),
        Op::Sync => {}
    }
}

/// Sends pending updates both ways, recording them for a late replica.
fn exchange(a: &mut Board, b: &mut Board, log: &mut Vec<Vec<u8>>) {
    if let Some(u) = a.take_pending_update().unwrap() {
        b.import(&u).unwrap();
        log.push(u);
    }
    if let Some(u) = b.take_pending_update().unwrap() {
        a.import(&u).unwrap();
        log.push(u);
    }
}

/// Two replicas with fixed peer ids (deterministic tie-breaks).
fn pair() -> (Board, Board) {
    let a = Board::new();
    let b = Board::new();
    a.set_peer_id(1);
    b.set_peer_id(2);
    (a, b)
}

/// A replica that only sees the log at the end, in either order.
fn late_replica(mut log: Vec<Vec<u8>>, reversed: bool) -> Board {
    let mut late = Board::new();
    if reversed {
        log.reverse();
    }
    for u in &log {
        late.import(u).unwrap();
    }
    late
}

proptest! {
    #[test]
    fn replicas_converge(
        ops in prop::collection::vec((any::<bool>(), op()), 1..60),
        late_order_reversed in any::<bool>(),
    ) {
        let (mut a, mut b) = pair();
        let mut log = Vec::new();

        for (on_a, op) in &ops {
            if matches!(op, Op::Sync) {
                exchange(&mut a, &mut b, &mut log);
            } else if *on_a {
                apply(&a, op);
            } else {
                apply(&b, op);
            }
        }
        exchange(&mut a, &mut b, &mut log);
        exchange(&mut a, &mut b, &mut log);

        prop_assert_eq!(a.shapes(), b.shapes());
        prop_assert_eq!(late_replica(log, late_order_reversed).shapes(), a.shapes());
    }
}

/// One textarea edit: replace `del` characters at `pos` (both taken modulo
/// the current length) with `insert`, then hand the whole new value to
/// `set_text`, exactly as the browser does on every `input` event.
#[derive(Debug, Clone)]
struct Edit {
    on_a: bool,
    pos: usize,
    del: usize,
    insert: String,
    sync_after: bool,
}

fn edit() -> impl Strategy<Value = Edit> {
    (
        any::<bool>(),
        any::<usize>(),
        0usize..4,
        "[a-zé😀 \n]{0,4}",
        prop::bool::weighted(0.2),
    )
        .prop_map(|(on_a, pos, del, insert, sync_after)| Edit {
            on_a,
            pos,
            del,
            insert,
            sync_after,
        })
}

fn apply_edit(board: &Board, e: &Edit) {
    let old: Vec<char> = board.text("p").unwrap().chars().collect();
    let pos = e.pos % (old.len() + 1);
    let end = (pos + e.del).min(old.len());
    let new: String = old[..pos]
        .iter()
        .copied()
        .chain(e.insert.chars())
        .chain(old[end..].iter().copied())
        .collect();
    board.set_text("p", &new).unwrap();
}

proptest! {
    #[test]
    fn concurrent_text_edits_converge(
        edits in prop::collection::vec(edit(), 1..40),
        late_order_reversed in any::<bool>(),
    ) {
        let (mut a, mut b) = pair();
        let mut log = Vec::new();
        a.add_postit("p", 0.0, 0.0, 200.0, 200.0, Color::Yellow).unwrap();
        exchange(&mut a, &mut b, &mut log);

        for e in &edits {
            apply_edit(if e.on_a { &a } else { &b }, e);
            if e.sync_after {
                exchange(&mut a, &mut b, &mut log);
            }
        }
        exchange(&mut a, &mut b, &mut log);

        let text = a.text("p").unwrap();
        prop_assert_eq!(&text, &b.text("p").unwrap());
        prop_assert_eq!(&late_replica(log, late_order_reversed).text("p").unwrap(), &text);
    }

    /// Without deletions nothing typed is ever lost: the merged text has
    /// exactly as many characters as both users typed, and each user's own
    /// characters appear in the order they typed them.
    #[test]
    fn concurrent_insertions_keep_every_character(
        a_typed in "[a-m]{1,12}",
        b_typed in "[n-z]{1,12}",
        a_pos in any::<usize>(),
        b_pos in any::<usize>(),
    ) {
        let (mut a, mut b) = pair();
        let mut log = Vec::new();
        a.add_postit("p", 0.0, 0.0, 200.0, 200.0, Color::Yellow).unwrap();
        exchange(&mut a, &mut b, &mut log);

        // Each user appends their characters at the end, one keystroke at a
        // time, while the other user does the same without syncing.
        for (board, typed) in [(&a, &a_typed), (&b, &b_typed)] {
            for c in typed.chars() {
                let mut text = board.text("p").unwrap();
                text.push(c);
                board.set_text("p", &text).unwrap();
            }
        }
        // Then each inserts once more at arbitrary positions, still offline.
        for (board, pos, c) in [(&a, a_pos, 'A'), (&b, b_pos, 'B')] {
            let mut text: Vec<char> = board.text("p").unwrap().chars().collect();
            text.insert(pos % (text.len() + 1), c);
            board.set_text("p", &text.into_iter().collect::<String>()).unwrap();
        }
        exchange(&mut a, &mut b, &mut log);

        let merged = a.text("p").unwrap();
        prop_assert_eq!(&merged, &b.text("p").unwrap());
        prop_assert_eq!(merged.chars().count(), a_typed.len() + b_typed.len() + 2);
        let only = |range: std::ops::RangeInclusive<char>| -> String {
            merged.chars().filter(|c| range.contains(c)).collect()
        };
        prop_assert_eq!(only('a'..='m'), a_typed);
        prop_assert_eq!(only('n'..='z'), b_typed);
    }
}
