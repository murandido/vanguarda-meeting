use std::f64::consts::PI;

use loro::LoroMap;

use super::*;
use crate::shape::{MAX_TEXT_CHARS, POSTIT_MAX_SIZE, POSTIT_MIN_SIZE, STICKER_MAX_SCALE};

fn postit(board: &Board, id: &str, x: f64, y: f64, w: f64, h: f64) {
    board.add_postit(id, x, y, w, h, Color::Yellow).unwrap();
}

/// Sends every pending change of `from` to `to`.
fn sync(from: &mut Board, to: &mut Board) {
    if let Some(update) = from.take_pending_update().unwrap() {
        to.import(&update).unwrap();
    }
}

fn postit_body(board: &Board, id: &str) -> (f64, f64, Color, String) {
    match board.shape(id).unwrap().body {
        Body::Postit { w, h, color, text } => (w, h, color, text),
        Body::Sticker { .. } => panic!("{id} is a sticker"),
    }
}

fn sticker_body(board: &Board, id: &str) -> (usize, f64, f64) {
    match board.shape(id).unwrap().body {
        Body::Sticker {
            emoji,
            scale,
            rotation,
        } => (emoji, scale, rotation),
        Body::Postit { .. } => panic!("{id} is a post-it"),
    }
}

#[test]
fn add_and_move_postit() {
    let board = Board::new();
    postit(&board, "a", 0.0, 0.0, 100.0, 80.0);
    board.move_shape("a", 5.0, 6.0).unwrap();
    assert_eq!(
        board.shapes(),
        vec![Shape {
            id: "a".into(),
            x: 5.0,
            y: 6.0,
            body: Body::Postit {
                w: 100.0,
                h: 80.0,
                color: Color::Yellow,
                text: String::new(),
            },
        }]
    );
}

#[test]
fn operations_on_unknown_shapes_fail() {
    let board = Board::new();
    assert!(matches!(
        board.move_shape("nada", 1.0, 1.0),
        Err(BoardError::UnknownShape(_))
    ));
    assert!(matches!(
        board.set_text("nada", "x"),
        Err(BoardError::UnknownShape(_))
    ));
    assert!(matches!(
        board.transform_sticker("nada", 1.0, 0.0),
        Err(BoardError::UnknownShape(_))
    ));
    assert!(board.shape("nada").is_none());
}

#[test]
fn operations_check_the_shape_kind() {
    let board = Board::new();
    postit(&board, "p", 0.0, 0.0, 100.0, 100.0);
    board.add_sticker("s", "🔥", 0.0, 0.0).unwrap();
    assert!(matches!(
        board.transform_sticker("p", 2.0, 0.0),
        Err(BoardError::WrongKind(_))
    ));
    assert!(matches!(
        board.resize_postit("s", 10.0, 10.0),
        Err(BoardError::WrongKind(_))
    ));
    assert!(matches!(
        board.set_color("s", Color::Blue),
        Err(BoardError::WrongKind(_))
    ));
    assert!(matches!(
        board.set_text("s", "x"),
        Err(BoardError::WrongKind(_))
    ));
    assert!(matches!(board.text("s"), Err(BoardError::WrongKind(_))));
}

#[test]
fn non_finite_numbers_are_rejected() {
    let board = Board::new();
    assert!(matches!(
        board.add_postit("p", f64::NAN, 0.0, 10.0, 10.0, Color::Yellow),
        Err(BoardError::NotFinite)
    ));
    assert!(matches!(
        board.add_sticker("s", "🔥", 0.0, f64::INFINITY),
        Err(BoardError::NotFinite)
    ));
    postit(&board, "p", 0.0, 0.0, 100.0, 100.0);
    assert!(matches!(
        board.move_shape("p", f64::NAN, 0.0),
        Err(BoardError::NotFinite)
    ));
    assert!(matches!(
        board.resize_postit("p", f64::INFINITY, 1.0),
        Err(BoardError::NotFinite)
    ));
    board.add_sticker("s", "🔥", 0.0, 0.0).unwrap();
    assert!(matches!(
        board.transform_sticker("s", 1.0, f64::NAN),
        Err(BoardError::NotFinite)
    ));
    assert_eq!(board.shapes().len(), 2);
}

#[test]
fn postit_size_is_clamped() {
    let board = Board::new();
    postit(&board, "p", 0.0, 0.0, 1.0, 1e6);
    let (w, h, ..) = postit_body(&board, "p");
    assert_eq!((w, h), (POSTIT_MIN_SIZE, POSTIT_MAX_SIZE));
}

#[test]
fn resize_keeps_the_corner_and_clamps() {
    let board = Board::new();
    postit(&board, "p", 10.0, 20.0, 100.0, 100.0);
    board.resize_postit("p", 250.0, 5.0).unwrap();
    let shape = board.shape("p").unwrap();
    assert_eq!((shape.x, shape.y), (10.0, 20.0));
    let (w, h, ..) = postit_body(&board, "p");
    assert_eq!((w, h), (250.0, POSTIT_MIN_SIZE));
}

#[test]
fn colors_are_stored_by_name() {
    let board = Board::new();
    postit(&board, "p", 0.0, 0.0, 100.0, 100.0);
    board.set_color("p", Color::Purple).unwrap();
    assert_eq!(postit_body(&board, "p").2, Color::Purple);
    let fields = board.shape_map().get_deep_value();
    let LoroValue::Map(fields) = fields else {
        panic!()
    };
    let LoroValue::Map(p) = &fields["p"] else {
        panic!()
    };
    assert_eq!(p["color"], LoroValue::from("purple"));
}

#[test]
fn unknown_colors_from_peers_fall_back_to_yellow() {
    let board = Board::new();
    postit(&board, "p", 0.0, 0.0, 100.0, 100.0);
    let (shape, _) = board.shape_entry("p").unwrap();
    shape.insert("color", "magenta").unwrap();
    board.doc.commit();
    assert_eq!(postit_body(&board, "p").2, Color::Yellow);
}

#[test]
fn text_is_set_read_and_truncated() {
    let board = Board::new();
    postit(&board, "p", 0.0, 0.0, 100.0, 100.0);
    assert_eq!(board.text("p").unwrap(), "");
    board.set_text("p", "olá\nmundo").unwrap();
    assert_eq!(board.text("p").unwrap(), "olá\nmundo");
    assert_eq!(postit_body(&board, "p").3, "olá\nmundo");
    board.set_text("p", "olá").unwrap();
    assert_eq!(board.text("p").unwrap(), "olá");
    board
        .set_text("p", &"x".repeat(MAX_TEXT_CHARS * 2))
        .unwrap();
    assert_eq!(board.text("p").unwrap().len(), MAX_TEXT_CHARS);
}

#[test]
fn setting_the_same_text_records_nothing() {
    let mut board = Board::new();
    postit(&board, "p", 0.0, 0.0, 100.0, 100.0);
    board.set_text("p", "igual").unwrap();
    board.take_pending_update().unwrap();
    board.set_text("p", "igual").unwrap();
    assert!(board.take_pending_update().unwrap().is_none());
}

#[test]
fn concurrent_text_edits_merge_character_by_character() {
    let mut a = Board::new();
    let mut b = Board::new();
    postit(&a, "p", 0.0, 0.0, 100.0, 100.0);
    a.set_text("p", "hello").unwrap();
    sync(&mut a, &mut b);

    // Both edit at the same time, in different places.
    a.set_text("p", "hello world").unwrap();
    b.set_text("p", "Hi, hello").unwrap();
    sync(&mut a, &mut b);
    sync(&mut b, &mut a);

    assert_eq!(a.text("p").unwrap(), "Hi, hello world");
    assert_eq!(b.text("p").unwrap(), "Hi, hello world");
}

#[test]
fn concurrent_move_and_recolor_both_survive() {
    let mut a = Board::new();
    let mut b = Board::new();
    postit(&a, "p", 0.0, 0.0, 100.0, 100.0);
    sync(&mut a, &mut b);
    a.move_shape("p", 50.0, 60.0).unwrap();
    b.set_color("p", Color::Green).unwrap();
    b.resize_postit("p", 200.0, 150.0).unwrap();
    sync(&mut a, &mut b);
    sync(&mut b, &mut a);
    assert_eq!(a.shapes(), b.shapes());
    let shape = a.shape("p").unwrap();
    assert_eq!((shape.x, shape.y), (50.0, 60.0));
    assert_eq!(
        postit_body(&a, "p"),
        (200.0, 150.0, Color::Green, String::new())
    );
}

#[test]
fn legacy_rects_load_as_yellow_postits() {
    let legacy = Board::new();
    legacy.add_rect("r", 1.0, 2.0, 120.0, 80.0);
    let snapshot = legacy.snapshot().unwrap();

    let board = Board::from_snapshot(&snapshot).unwrap();
    assert_eq!(
        board.shapes(),
        vec![Shape {
            id: "r".into(),
            x: 1.0,
            y: 2.0,
            body: Body::Postit {
                w: 120.0,
                h: 80.0,
                color: Color::Yellow,
                text: String::new(),
            },
        }]
    );
    assert_eq!(board.text("r").unwrap(), "");

    // And they can be edited like any post-it.
    board.set_text("r", "antigo").unwrap();
    board.set_color("r", Color::Pink).unwrap();
    board.resize_postit("r", 200.0, 200.0).unwrap();
    assert_eq!(
        postit_body(&board, "r"),
        (200.0, 200.0, Color::Pink, "antigo".into())
    );
}

#[test]
fn concurrent_typing_into_a_legacy_rect_keeps_both_edits() {
    let mut a = Board::new();
    let mut b = Board::new();
    a.add_rect("r", 0.0, 0.0, 120.0, 80.0);
    sync(&mut a, &mut b);
    // Neither replica has a text container yet; both create one at once.
    a.set_text("r", "ana").unwrap();
    b.set_text("r", "bia").unwrap();
    sync(&mut a, &mut b);
    sync(&mut b, &mut a);
    let merged = a.text("r").unwrap();
    assert_eq!(merged, b.text("r").unwrap());
    assert_eq!(merged.len(), 6);
    assert!(merged.contains("ana") && merged.contains("bia"));
}

#[test]
fn malformed_entries_are_skipped() {
    let board = Board::new();
    let map = board.shape_map();
    map.insert("scalar", 1.0).unwrap();
    let no_coords = map.insert_container("no-coords", LoroMap::new()).unwrap();
    no_coords.insert("w", 10.0).unwrap();
    let alien = map.insert_container("alien", LoroMap::new()).unwrap();
    alien.insert("kind", "circle").unwrap();
    alien.insert("x", 0.0).unwrap();
    alien.insert("y", 0.0).unwrap();
    let bad_emoji = map.insert_container("bad-emoji", LoroMap::new()).unwrap();
    bad_emoji.insert("kind", "sticker").unwrap();
    bad_emoji.insert("emoji", "🦀").unwrap();
    bad_emoji.insert("x", 0.0).unwrap();
    bad_emoji.insert("y", 0.0).unwrap();
    board.doc.commit();
    assert_eq!(board.shapes(), Vec::new());
    assert!(matches!(board.text("alien"), Err(BoardError::WrongKind(_))));
}

#[test]
fn stickers_are_stamped_and_transformed() {
    let board = Board::new();
    assert!(matches!(
        board.add_sticker("s", "🦀", 0.0, 0.0),
        Err(BoardError::UnknownSticker(_))
    ));
    board.add_sticker("s", "🚀", 10.0, 20.0).unwrap();
    let rocket = STICKERS.iter().position(|e| *e == "🚀").unwrap();
    assert_eq!(sticker_body(&board, "s"), (rocket, 1.0, 0.0));

    board.transform_sticker("s", 2.5, PI / 2.0).unwrap();
    assert_eq!(sticker_body(&board, "s"), (rocket, 2.5, PI / 2.0));

    // Scale is clamped; rotation is normalized to (-π, π].
    board.transform_sticker("s", 100.0, 3.0 * PI / 2.0).unwrap();
    let (_, scale, rotation) = sticker_body(&board, "s");
    assert_eq!(scale, STICKER_MAX_SCALE);
    assert!((rotation + PI / 2.0).abs() < 1e-9);

    board.move_shape("s", -5.0, -6.0).unwrap();
    let shape = board.shape("s").unwrap();
    assert_eq!((shape.x, shape.y), (-5.0, -6.0));
}

#[test]
fn stickers_draw_above_postits_and_newer_ids_above_older() {
    let board = Board::new();
    board.add_sticker("0-sticker", "⭐", 50.0, 50.0).unwrap();
    postit(&board, "2-new", 0.0, 0.0, 100.0, 100.0);
    postit(&board, "1-old", 0.0, 0.0, 100.0, 100.0);
    let ids: Vec<_> = board.shapes().into_iter().map(|s| s.id).collect();
    assert_eq!(ids, ["1-old", "2-new", "0-sticker"]);
    assert_eq!(board.hit_test(50.0, 50.0).unwrap().id, "0-sticker");
    assert_eq!(board.hit_test(10.0, 10.0).unwrap().id, "2-new");
}

#[test]
fn remove_deletes_any_shape() {
    let board = Board::new();
    postit(&board, "p", 0.0, 0.0, 100.0, 100.0);
    board.add_sticker("s", "⭐", 0.0, 0.0).unwrap();
    board.remove_shape("s");
    board.remove_shape("p");
    assert_eq!(board.shapes(), Vec::new());
}

#[test]
fn pending_update_is_taken_once() {
    let mut board = Board::new();
    assert!(board.take_pending_update().unwrap().is_none());
    postit(&board, "a", 0.0, 0.0, 100.0, 100.0);
    assert!(board.take_pending_update().unwrap().is_some());
    assert!(board.take_pending_update().unwrap().is_none());
}

#[test]
fn snapshot_restores_state() {
    let board = Board::new();
    postit(&board, "a", 1.0, 2.0, 300.0, 400.0);
    board.set_text("a", "texto").unwrap();
    board.add_sticker("b", "🎉", 5.0, 5.0).unwrap();
    let restored = Board::from_snapshot(&board.snapshot().unwrap()).unwrap();
    assert_eq!(restored.shapes(), board.shapes());
    assert!(matches!(
        Board::from_snapshot(&[0xff; 16]),
        Err(BoardError::Import(_))
    ));
}

#[test]
fn cache_follows_local_and_remote_changes() {
    let mut a = Board::new();
    let mut b = Board::new();
    postit(&a, "a", 0.0, 0.0, 100.0, 100.0);
    assert_eq!(a.shapes().len(), 1);
    a.move_shape("a", 5.0, 5.0).unwrap();
    assert_eq!(a.shapes()[0].x, 5.0);
    postit(&b, "b", 1.0, 1.0, 100.0, 100.0);
    sync(&mut b, &mut a);
    assert_eq!(a.shapes().len(), 2);
    a.remove_shape("a");
    assert_eq!(
        a.shapes().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["b"]
    );
}

#[test]
fn visible_records_cull_to_viewport() {
    let board = Board::new();
    postit(&board, "a", 0.0, 0.0, 50.0, 50.0);
    postit(&board, "b", 500.0, 500.0, 50.0, 50.0);
    board.add_sticker("c", "⭐", -100.0, -100.0).unwrap();
    board.set_text("a", "oi").unwrap();

    let view = Camera::default().viewport(100.0, 100.0);
    let records = board.visible_records(&view);
    assert_eq!(records.len(), RECORD);
    assert_eq!(records[..7], [0.0, 0.0, 0.0, 0.0, 50.0, 50.0, 0.0]);
    assert_eq!(records[7], shape::text_version("oi"));
    assert_eq!(board.text_at(0), "oi");

    let far = Camera::new(400.0, 400.0, 1.0).viewport(200.0, 200.0);
    assert_eq!(board.visible_records(&far)[..2], [1.0, 0.0]);

    let all = Camera::new(-200.0, -200.0, 0.1).viewport(10_000.0, 10_000.0);
    let records = board.visible_records(&all);
    assert_eq!(records.len(), 3 * RECORD);
    // The sticker is drawn last and carries its emoji index.
    assert_eq!(records[2 * RECORD..2 * RECORD + 2], [2.0, 1.0]);
    assert_eq!(board.text_at(2), "");
    assert_eq!(board.text_at(99), "");
}

#[test]
fn cull_keeps_draw_order() {
    let board = Board::new();
    for i in 0..5 {
        postit(
            &board,
            &format!("r{i}"),
            f64::from(i) * 100.0,
            0.0,
            50.0,
            50.0,
        );
    }
    let view = Bounds::from_rect(160.0, 10.0, 200.0, 10.0);
    let ids: Vec<_> =
        board.with_shapes(|shapes| cull(shapes, &view).map(|s| s.id.clone()).collect());
    assert_eq!(ids, ["r2", "r3"]);
}

#[test]
fn hit_test_returns_topmost() {
    let board = Board::new();
    assert!(board.hit_test(0.0, 0.0).is_none());
    postit(&board, "a", 0.0, 0.0, 100.0, 100.0);
    postit(&board, "b", 50.0, 50.0, 100.0, 100.0);
    assert_eq!(board.hit_test(10.0, 10.0).unwrap().id, "a");
    assert_eq!(board.hit_test(60.0, 60.0).unwrap().id, "b");
    assert!(board.hit_test(200.0, 10.0).is_none());
}

#[test]
fn content_bounds_wraps_everything() {
    let board = Board::new();
    assert!(board.content_bounds().is_none());
    postit(&board, "a", -10.0, 5.0, 50.0, 50.0);
    board.add_sticker("b", "⭐", 200.0, -40.0).unwrap();
    let b = board.content_bounds().unwrap();
    assert_eq!((b.min_x, b.max_y), (-10.0, 55.0));
    // The sticker contributes its rotation-safe box (half side · √2 ≈ 45.25).
    assert!((b.max_x - 245.25).abs() < 0.01);
    assert!((b.min_y + 85.25).abs() < 0.01);
}
