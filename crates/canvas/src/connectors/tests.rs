use super::*;
use crate::Camera;

const EPS: f64 = 1e-9;

fn close(a: Point, b: Point) -> bool {
    (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS
}

/// Two 100x100 shapes side by side: `a` at the origin, `b` 300 to the right.
fn two_shapes() -> Board {
    let board = Board::new();
    board.add_rect("a", 0.0, 0.0, 100.0, 100.0);
    board.add_rect("b", 300.0, 0.0, 100.0, 100.0);
    board
}

fn link(board: &Board, id: &str, from: &str, to: &str) {
    board.add_connector(
        id,
        &Endpoint::shape(from, None),
        &Endpoint::shape(to, None),
        Route::Curved,
    );
}

#[test]
fn anchors_sit_on_side_midpoints() {
    let b = Bounds::from_rect(10.0, 20.0, 100.0, 50.0);
    assert_eq!(Side::N.anchor(&b), (60.0, 20.0));
    assert_eq!(Side::E.anchor(&b), (110.0, 45.0));
    assert_eq!(Side::S.anchor(&b), (60.0, 70.0));
    assert_eq!(Side::W.anchor(&b), (10.0, 45.0));
    for side in Side::ALL {
        assert_eq!(Side::parse(Side::as_str(Some(side))), Some(side));
    }
    assert_eq!(Side::parse("auto"), None);
}

#[test]
fn auto_side_faces_the_other_end() {
    let b = Bounds::from_rect(0.0, 0.0, 100.0, 100.0);
    assert_eq!(Side::facing(&b, (500.0, 60.0)), Side::E);
    assert_eq!(Side::facing(&b, (-500.0, 60.0)), Side::W);
    assert_eq!(Side::facing(&b, (40.0, 400.0)), Side::S);
    assert_eq!(Side::facing(&b, (40.0, -400.0)), Side::N);
    // Each axis is weighted by the box size: 300 to the right of a 1000-wide
    // shape is "less" than 200 below a 100-tall one.
    let wide = Bounds::from_rect(0.0, 0.0, 1000.0, 100.0);
    assert_eq!(Side::facing(&wide, (800.0, 250.0)), Side::S);
}

#[test]
fn straight_route_is_a_segment() {
    let c = route(
        ((0.0, 0.0), Some((1.0, 0.0))),
        ((90.0, 30.0), Some((-1.0, 0.0))),
        Route::Straight,
    );
    assert!(close(c.c1, (30.0, 10.0)));
    assert!(close(c.c2, (60.0, 20.0)));
    assert!(close(c.point(0.5), (45.0, 15.0)));
    assert!(c.distance((45.0, 15.0)) < EPS);
    assert!(close(c.end_direction(), unit((90.0, 30.0)).unwrap()));
}

#[test]
fn curved_route_leaves_and_enters_along_side_normals() {
    let c = route(
        ((100.0, 50.0), Some((1.0, 0.0))),
        ((300.0, 50.0), Some((-1.0, 0.0))),
        Route::Curved,
    );
    // Pull = 0.5 * distance (200) = 100.
    assert!(close(c.c1, (200.0, 50.0)));
    assert!(close(c.c2, (200.0, 50.0)));
    assert!(close(c.point(0.0), (100.0, 50.0)));
    assert!(close(c.point(1.0), (300.0, 50.0)));
    // The arrowhead points into the target, against its outward normal.
    assert!(close(c.end_direction(), (1.0, 0.0)));

    // A curve from the bottom of one shape to the top of another, offset
    // sideways, bows: its midpoint is between the ends, the tangents vertical.
    let s = route(
        ((0.0, 0.0), Some((0.0, 1.0))),
        ((200.0, 400.0), Some((0.0, -1.0))),
        Route::Curved,
    );
    assert!(close(s.point(0.5), (100.0, 200.0)));
    assert!(close(s.end_direction(), (0.0, 1.0)));

    // Short distances still bow out by CURVE_MIN; long ones are capped.
    let short = route(
        ((0.0, 0.0), Some((0.0, -1.0))),
        ((10.0, 0.0), Some((0.0, -1.0))),
        Route::Curved,
    );
    assert!(close(short.c1, (0.0, -CURVE_MIN)));
    let long = route(
        ((0.0, 0.0), Some((0.0, 1.0))),
        ((0.0, 5000.0), Some((0.0, -1.0))),
        Route::Curved,
    );
    assert!(close(long.c1, (0.0, CURVE_MAX)));
    assert!(close(long.c2, (0.0, 5000.0 - CURVE_MAX)));
}

#[test]
fn free_end_still_has_a_direction() {
    let c = route(
        ((0.0, 0.0), Some((0.0, 1.0))),
        ((200.0, 0.0), None),
        Route::Curved,
    );
    let (dx, dy) = c.end_direction();
    assert!((dx.hypot(dy) - 1.0).abs() < EPS);
    // Fully degenerate curve: all four points equal.
    let p = (5.0, 5.0);
    let dot = Curve {
        p0: p,
        c1: p,
        c2: p,
        p3: p,
    };
    assert_eq!(dot.end_direction(), (1.0, 0.0));
    assert!((dot.distance((8.0, 9.0)) - 5.0).abs() < EPS);
}

#[test]
fn hit_test_follows_the_curve_not_its_box() {
    let board = Board::new();
    board.add_rect("a", 0.0, 0.0, 100.0, 100.0);
    board.add_rect("b", 300.0, 300.0, 100.0, 100.0);
    board.add_connector(
        "c",
        &Endpoint::shape("a", Some(Side::E)),
        &Endpoint::shape("b", Some(Side::N)),
        Route::Curved,
    );
    let curve = board.routed_connectors()[0].curve;
    let (mx, my) = curve.point(0.5);
    assert_eq!(board.hit_connector(mx, my, 1.0).as_deref(), Some("c"));
    assert_eq!(board.hit_connector(mx + 3.0, my, 4.0).as_deref(), Some("c"));
    // Inside the curve's box, but far from the curve itself.
    assert!(curve.bounds().contains(110.0, 290.0));
    assert_eq!(board.hit_connector(110.0, 290.0, 6.0), None);
    // The wasm layer turns screen pixels into world units: 6px at 400% is
    // 1.5 world units, too little to reach a point 3 units away.
    assert_eq!(board.hit_connector(mx + 3.0, my, 6.0 / 4.0), None);
}

#[test]
fn topmost_connector_wins() {
    let board = two_shapes();
    link(&board, "c1", "a", "b");
    link(&board, "c2", "a", "b");
    assert_eq!(board.hit_connector(200.0, 50.0, 2.0).as_deref(), Some("c2"));
}

#[test]
fn arrow_follows_moving_shapes() {
    let board = two_shapes();
    link(&board, "c", "a", "b");
    let before = board.connector_geometry("c");
    assert_eq!(before.len(), STRIDE);
    assert_eq!(&before[..2], &[100.0, 50.0]); // a's east side
    assert_eq!(&before[6..8], &[300.0, 50.0]); // b's west side

    // Moving b below a flips the auto sides to S -> N.
    board.move_shape("b", 0.0, 400.0).unwrap();
    let after = board.connector_geometry("c");
    assert_eq!(&after[..2], &[50.0, 100.0]);
    assert_eq!(&after[6..8], &[50.0, 400.0]);
    assert!(close((after[8], after[9]), (0.0, 1.0)));
}

#[test]
fn remote_moves_bend_the_arrow_too() {
    let mut a = two_shapes();
    link(&a, "c", "a", "b");
    let mut b = Board::from_snapshot(&a.snapshot().unwrap()).unwrap();
    a.take_pending_update().unwrap();
    b.move_shape("b", 300.0, 200.0).unwrap();
    a.import(&b.take_pending_update().unwrap().unwrap())
        .unwrap();
    assert_eq!(a.connector_geometry("c"), b.connector_geometry("c"));
    assert_eq!(&a.connector_geometry("c")[6..8], &[300.0, 250.0]);
}

#[test]
fn cull_keeps_only_connectors_crossing_the_view() {
    let board = Board::new();
    board.add_rect("a", 0.0, 0.0, 10.0, 10.0);
    board.add_rect("b", 1000.0, 0.0, 10.0, 10.0);
    board.add_rect("far1", 5000.0, 5000.0, 10.0, 10.0);
    board.add_rect("far2", 6000.0, 5000.0, 10.0, 10.0);
    link(&board, "near", "a", "b");
    link(&board, "far", "far1", "far2");
    // The view sees neither a nor b, only the middle of the arrow.
    let view = Camera::new(400.0, -50.0, 1.0).viewport(200.0, 100.0);
    assert_eq!(
        board.visible_connectors(&view, 0.0),
        board.connector_geometry("near")
    );
    let all = Camera::new(-100.0, -100.0, 0.1).viewport(10_000.0, 10_000.0);
    assert_eq!(board.visible_connectors(&all, 0.0).len(), 2 * STRIDE);
    let nothing = Camera::new(-5000.0, -5000.0, 1.0).viewport(100.0, 100.0);
    assert_eq!(board.visible_connectors(&nothing, 0.0).len(), 0);
    // The margin lets an arrowhead just outside the view be drawn.
    // Shapes are at least 40 wide, so the arrow starts at x = 40.
    let edge = Camera::new(0.0, 0.0, 1.0).viewport(30.0, 20.0);
    assert_eq!(board.visible_connectors(&edge, 0.0).len(), 0);
    assert_eq!(board.visible_connectors(&edge, 16.0).len(), STRIDE);
}

#[test]
fn connectors_are_not_drawable_shapes() {
    let board = two_shapes();
    link(&board, "c", "a", "b");
    board.add_connector(
        "free",
        &Endpoint::shape("a", Some(Side::S)),
        &Endpoint::Point { x: 50.0, y: 900.0 },
        Route::Straight,
    );
    assert_eq!(board.shapes().len(), 2);
    assert_eq!(board.hit_test(200.0, 50.0), None);
    assert_eq!(
        board.content_bounds(),
        Some(Bounds::from_rect(0.0, 0.0, 400.0, 100.0))
    );
    let free = board.connector_geometry("free");
    assert_eq!(&free[..2], &[50.0, 100.0]);
    assert_eq!(&free[6..8], &[50.0, 900.0]);
}

#[test]
fn dangling_connectors_are_kept_but_not_drawn() {
    let board = two_shapes();
    link(&board, "c", "a", "missing");
    link(&board, "d", "c", "a"); // a connector is not a valid target
    assert_eq!(board.connectors().len(), 2);
    assert_eq!(board.routed_connectors().len(), 0);
    assert_eq!(board.connector_geometry("c").len(), 0);
    assert_eq!(board.hit_connector(200.0, 50.0, 1000.0), None);
    let view = Camera::default().viewport(1000.0, 1000.0);
    assert_eq!(board.visible_connectors(&view, 0.0).len(), 0);
    // If the shape shows up again (e.g. a future undo), so does the arrow.
    board.add_rect("missing", 300.0, 300.0, 10.0, 10.0);
    assert_eq!(board.routed_connectors().len(), 1);
}

#[test]
fn removing_a_shape_removes_its_connectors() {
    let board = two_shapes();
    board.add_rect("z", 0.0, 500.0, 10.0, 10.0);
    link(&board, "ab", "a", "b");
    link(&board, "ba", "b", "a");
    link(&board, "bz", "b", "z");
    board.remove_shape("a");
    let ids: Vec<_> = board.connectors().into_iter().map(|c| c.id).collect();
    assert_eq!(ids, ["bz"]);
    // Removing a connector keeps its shapes.
    board.remove_shape("bz");
    assert_eq!(board.connectors().len(), 0);
    assert_eq!(board.shapes().len(), 2);
}

#[test]
fn route_toggle_and_snapshot_round_trip() {
    let board = two_shapes();
    link(&board, "c", "a", "b");
    assert!(board.set_connector_route("c", Route::Straight));
    assert!(!board.set_connector_route("a", Route::Straight));
    assert!(!board.set_connector_route("nope", Route::Straight));
    let restored = Board::from_snapshot(&board.snapshot().unwrap()).unwrap();
    assert_eq!(restored.connectors(), board.connectors());
    assert_eq!(restored.connectors()[0].route, Route::Straight);
    let g = restored.connector_geometry("c");
    assert!(close(
        (g[2], g[3]),
        lerp((100.0, 50.0), (300.0, 50.0), 1.0 / 3.0)
    ));
}

#[test]
fn preview_and_anchors() {
    let board = two_shapes();
    assert_eq!(
        board.anchor_points("a"),
        vec![50.0, 0.0, 100.0, 50.0, 50.0, 100.0, 0.0, 50.0]
    );
    assert_eq!(board.anchor_points("nope").len(), 0);
    assert_eq!(
        board.anchor_near(101.0, 52.0, 4.0),
        Some(("a".into(), Side::E))
    );
    assert_eq!(
        board.anchor_near(299.0, 50.0, 4.0),
        Some(("b".into(), Side::W))
    );
    assert_eq!(board.anchor_near(200.0, 50.0, 4.0), None);
    let p = board.preview_connector(
        &Endpoint::shape("a", Some(Side::S)),
        &Endpoint::Point { x: 50.0, y: 300.0 },
        Route::Curved,
    );
    assert_eq!(&p[..2], &[50.0, 100.0]);
    assert_eq!(&p[6..8], &[50.0, 300.0]);
    let missing = board.preview_connector(
        &Endpoint::shape("x", None),
        &Endpoint::Point { x: 0.0, y: 0.0 },
        Route::Curved,
    );
    assert_eq!(missing.len(), 0);
}

#[test]
fn malformed_connector_values_are_ignored() {
    let board = two_shapes();
    let map = board
        .shape_map()
        .insert_container("bad", LoroMap::new())
        .unwrap();
    map.insert("kind", KIND).unwrap();
    map.insert("from", "not a map").unwrap();
    map.insert("to", Endpoint::shape("b", None).to_value())
        .unwrap();
    board.doc.commit();
    assert_eq!(board.connectors().len(), 0);
    assert_eq!(board.shapes().len(), 2);
}

/// Two participants creating and deleting shapes and connectors concurrently,
/// syncing at arbitrary points, must converge to the same board without
/// panics, and never draw a connector whose shape is gone.
mod convergence {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Op {
        AddShape(u8),
        MoveShape(u8, i16),
        RemoveShape(u8),
        Connect(u8, u8, u8),
        ConnectToPoint(u8, u8),
        RemoveConnector(u8),
        Toggle(u8),
        Sync,
    }

    fn op() -> impl Strategy<Value = Op> {
        let s = 0u8..6;
        prop_oneof![
            s.clone().prop_map(Op::AddShape),
            (s.clone(), any::<i16>()).prop_map(|(i, d)| Op::MoveShape(i, d)),
            s.clone().prop_map(Op::RemoveShape),
            (0u8..4, s.clone(), s.clone()).prop_map(|(c, a, b)| Op::Connect(c, a, b)),
            (0u8..4, s).prop_map(|(c, a)| Op::ConnectToPoint(c, a)),
            (0u8..4).prop_map(Op::RemoveConnector),
            (0u8..4).prop_map(Op::Toggle),
            Just(Op::Sync),
        ]
    }

    fn has_shape(board: &Board, id: &str) -> bool {
        board.shapes().iter().any(|r| r.id == id)
    }

    fn apply(board: &Board, peer: u8, op: &Op) {
        let shape = |i: u8| format!("s{i}");
        // Connector ids are per peer so creations don't just overwrite each
        // other; shape ids are shared to provoke create/delete races.
        let conn = |i: u8| format!("c{peer}-{i}");
        match *op {
            Op::AddShape(i) => {
                board.add_rect(&shape(i), f64::from(i) * 200.0, 0.0, 100.0, 80.0);
            }
            Op::MoveShape(i, d) => {
                let _ = board.move_shape(&shape(i), f64::from(d), f64::from(d) / 2.0);
            }
            Op::RemoveShape(i) => board.remove_shape(&shape(i)),
            // The UI only connects shapes it can see; mimic that.
            Op::Connect(c, a, b) => {
                if a != b && has_shape(board, &shape(a)) && has_shape(board, &shape(b)) {
                    board.add_connector(
                        &conn(c),
                        &Endpoint::shape(&shape(a), Some(Side::ALL[usize::from(c)])),
                        &Endpoint::shape(&shape(b), None),
                        Route::Curved,
                    );
                }
            }
            Op::ConnectToPoint(c, a) => {
                if has_shape(board, &shape(a)) {
                    board.add_connector(
                        &conn(c),
                        &Endpoint::shape(&shape(a), None),
                        &Endpoint::Point { x: 0.0, y: 500.0 },
                        Route::Straight,
                    );
                }
            }
            Op::RemoveConnector(c) => board.remove_shape(&conn(c)),
            Op::Toggle(c) => {
                let _ = board.set_connector_route(&conn(c), Route::Straight);
            }
            Op::Sync => unreachable!("handled by the driver"),
        }
    }

    fn sync(a: &mut Board, b: &mut Board) {
        if let Some(u) = a.take_pending_update().unwrap() {
            b.import(&u).unwrap();
        }
        if let Some(u) = b.take_pending_update().unwrap() {
            a.import(&u).unwrap();
        }
    }

    fn check_invariants(board: &Board) {
        let all = board.connectors();
        let routed = board.routed_connectors();
        for r in &routed {
            let c = all.iter().find(|c| c.id == r.id).expect("routed is read");
            for end in [&c.from, &c.to] {
                if let Some(id) = end.shape_id() {
                    assert!(has_shape(board, id), "{} drawn without {id}", r.id);
                }
            }
            assert!(r.curve.flat().iter().all(|v| v.is_finite()));
            let (x, y) = r.curve.point(0.5);
            assert!(board.hit_connector(x, y, 0.5).is_some());
        }
        let view = Bounds::from_rect(-1e6, -1e6, 2e6, 2e6);
        assert_eq!(
            board.visible_connectors(&view, 0.0).len(),
            routed.len() * STRIDE
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]
        #[test]
        fn concurrent_edits_converge(ops in prop::collection::vec((any::<bool>(), op()), 1..60)) {
            let mut a = Board::new();
            let mut b = Board::new();
            a.set_peer_id(1);
            b.set_peer_id(2);
            for (on_a, op) in &ops {
                if matches!(op, Op::Sync) {
                    sync(&mut a, &mut b);
                } else if *on_a {
                    apply(&a, 1, op);
                } else {
                    apply(&b, 2, op);
                }
                check_invariants(&a);
                check_invariants(&b);
            }
            sync(&mut a, &mut b);
            prop_assert_eq!(a.shapes(), b.shapes());
            prop_assert_eq!(a.connectors(), b.connectors());
            prop_assert_eq!(a.routed_connectors(), b.routed_connectors());
            check_invariants(&a);
        }
    }

    #[test]
    fn connect_while_the_other_deletes_the_shape() {
        let mut a = Board::new();
        a.set_peer_id(1);
        a.add_rect("x", 0.0, 0.0, 10.0, 10.0);
        a.add_rect("y", 100.0, 0.0, 10.0, 10.0);
        let mut b = Board::from_snapshot(&a.snapshot().unwrap()).unwrap();
        b.set_peer_id(2);
        a.take_pending_update().unwrap();
        // Concurrently: A connects x -> y while B deletes y.
        a.add_connector(
            "c",
            &Endpoint::shape("x", None),
            &Endpoint::shape("y", None),
            Route::Curved,
        );
        b.remove_shape("y");
        sync(&mut a, &mut b);
        for board in [&a, &b] {
            assert_eq!(board.connectors().len(), 1);
            assert_eq!(board.routed_connectors().len(), 0);
            check_invariants(board);
        }
    }
}
