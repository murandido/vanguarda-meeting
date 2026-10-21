//! CA-01: custo de escolher o que desenhar a cada frame com 1.000 e 10.000
//! post-its. O orçamento de um frame a 60 fps é ~16,7 ms; o recorte precisa
//! ficar em microssegundos para sobrar tempo para o desenho.

use std::hint::black_box;

use canvas::{Board, Camera, Color, cull};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

/// Grade de post-its 120x80 com folga, como um quadro bem cheio.
fn board_with(n: u32) -> Board {
    let board = Board::new();
    let cols = 100;
    for i in 0..n {
        let x = f64::from(i % cols) * 160.0;
        let y = f64::from(i / cols) * 120.0;
        board
            .add_postit(&format!("r{i:05}"), x, y, 120.0, 80.0, Color::Yellow)
            .unwrap();
    }
    board
}

fn culling(c: &mut Criterion) {
    let mut group = c.benchmark_group("culling");
    for n in [1_000, 10_000] {
        let board = board_with(n);
        let shapes = board.shapes();
        // Tela 1280x800 em 100%, no meio do quadro: só uma fração visível.
        let view = Camera::new(4000.0, 200.0, 1.0).viewport(1280.0, 800.0);
        // Zoom mínimo: quase tudo visível (pior caso do desenho).
        let zoomed_out = Camera::new(0.0, 0.0, 0.1).viewport(1280.0, 800.0);

        group.bench_with_input(BenchmarkId::new("cull_100pct", n), &shapes, |b, shapes| {
            b.iter(|| cull(black_box(shapes), black_box(&view)).count());
        });
        group.bench_with_input(
            BenchmarkId::new("visible_records_100pct", n),
            &board,
            |b, board| b.iter(|| board.visible_records(black_box(&view))),
        );
        group.bench_with_input(
            BenchmarkId::new("visible_records_10pct", n),
            &board,
            |b, board| b.iter(|| board.visible_records(black_box(&zoomed_out))),
        );
    }
    group.finish();
}

criterion_group!(benches, culling);
criterion_main!(benches);
