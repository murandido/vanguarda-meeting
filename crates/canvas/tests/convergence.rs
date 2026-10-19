//! QG-04: réplicas que recebem as mesmas atualizações, em qualquer ordem,
//! terminam com o mesmo quadro.

use canvas::Board;
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    Add(u8, f64, f64),
    Move(u8, f64, f64),
    Remove(u8),
    /// Troca as atualizações pendentes entre as duas réplicas.
    Sync,
}

fn op() -> impl Strategy<Value = Op> {
    let id = 0u8..4;
    let coord = -1000.0..1000.0f64;
    prop_oneof![
        (id.clone(), coord.clone(), coord.clone()).prop_map(|(i, x, y)| Op::Add(i, x, y)),
        (id.clone(), coord.clone(), coord).prop_map(|(i, x, y)| Op::Move(i, x, y)),
        id.prop_map(Op::Remove),
        Just(Op::Sync),
    ]
}

fn apply(board: &Board, op: &Op) {
    match *op {
        Op::Add(i, x, y) => board.add_rect(&format!("r{i}"), x, y, 10.0, 10.0),
        // Mover ou remover algo que o outro já apagou é permitido no CRDT.
        Op::Move(i, x, y) => {
            let _ = board.move_shape(&format!("r{i}"), x, y);
        }
        Op::Remove(i) => board.remove_shape(&format!("r{i}")),
        Op::Sync => {}
    }
}

proptest! {
    #[test]
    fn replicas_converge(
        ops in prop::collection::vec((any::<bool>(), op()), 1..60),
        late_order_reversed in any::<bool>(),
    ) {
        let mut a = Board::new();
        let mut b = Board::new();
        a.set_peer_id(1);
        b.set_peer_id(2);
        // Tudo que cada réplica enviou, para uma terceira réplica que chega atrasada.
        let mut log = Vec::new();

        let exchange = |a: &mut Board, b: &mut Board, log: &mut Vec<Vec<u8>>| {
            if let Some(u) = a.take_pending_update().unwrap() {
                b.import(&u).unwrap();
                log.push(u);
            }
            if let Some(u) = b.take_pending_update().unwrap() {
                a.import(&u).unwrap();
                log.push(u);
            }
        };

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

        prop_assert_eq!(a.rects(), b.rects());

        let mut late = Board::new();
        if late_order_reversed {
            log.reverse();
        }
        for u in &log {
            late.import(u).unwrap();
        }
        prop_assert_eq!(late.rects(), a.rects());
    }
}
