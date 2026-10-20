//! Sincronização ponta a ponta pelo servidor real: dois clientes WebSocket
//! com o mesmo motor `canvas` que roda no navegador.

use std::time::Duration;

use canvas::Board;
use futures_util::{SinkExt, StreamExt};
use server::db;
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::header},
};

mod common;

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct Server {
    base: String,
    pool: SqlitePool,
}

async fn start(pool: SqlitePool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = common::router(pool.clone(), common::dev_auth());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Server {
        base: format!("ws://{addr}/ws"),
        pool,
    }
}

/// Cria uma sessão direto no banco e devolve o cookie.
async fn session(pool: &SqlitePool, login: &str) -> String {
    let id = format!("sessao-{login}");
    db::upsert_user(pool, login, None, None, None)
        .await
        .unwrap();
    let _ = db::create_session(pool, &id, login, i64::MAX).await;
    format!("vanguarda_session={id}")
}

async fn next_binary(ws: &mut Ws) -> Vec<u8> {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("mensagem em até 5 s")
            .unwrap()
            .unwrap();
        if let Message::Binary(bytes) = msg {
            return bytes.to_vec();
        }
    }
}

async fn join(server: &Server, board: &str, login: &str) -> (Ws, Board) {
    let mut req = format!("{}/{board}", server.base)
        .into_client_request()
        .unwrap();
    let cookie = session(&server.pool, login).await;
    req.headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    let (mut ws, _) = connect_async(req).await.unwrap();
    let snapshot = next_binary(&mut ws).await;
    let mut local = Board::new();
    local.import(&snapshot).unwrap();
    (ws, local)
}

async fn push(ws: &mut Ws, board: &mut Board) {
    let update = board.take_pending_update().unwrap().unwrap();
    ws.send(Message::Binary(update.into())).await.unwrap();
}

#[tokio::test]
async fn edits_reach_the_other_client() {
    let base = start(db::connect("sqlite::memory:").await.unwrap()).await;
    let (mut ws_a, mut a) = join(&base, "q1", "ana").await;
    let (mut ws_b, mut b) = join(&base, "q1", "bia").await;

    a.add_rect("r1", 10.0, 20.0, 100.0, 50.0);
    push(&mut ws_a, &mut a).await;
    b.import(&next_binary(&mut ws_b).await).unwrap();
    assert_eq!(b.rects(), a.rects());

    b.move_shape("r1", 300.0, 400.0).unwrap();
    push(&mut ws_b, &mut b).await;
    a.import(&next_binary(&mut ws_a).await).unwrap();
    assert_eq!(a.rects()[0].x, 300.0);
}

#[tokio::test]
async fn late_joiner_and_restart_get_the_saved_board() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    let base = start(pool.clone()).await;

    let (mut ws_a, mut a) = join(&base, "q2", "ana").await;
    a.add_rect("r1", 1.0, 2.0, 3.0, 4.0);
    push(&mut ws_a, &mut a).await;

    // Quem chega depois recebe o estado no snapshot inicial.
    let (_ws_b, b) = join(&base, "q2", "bia").await;
    assert_eq!(b.rects(), a.rects());

    // Um servidor novo sobre o mesmo banco também.
    drop(ws_a);
    let base2 = start(pool).await;
    let (_ws_c, c) = join(&base2, "q2", "ana").await;
    assert_eq!(c.rects(), a.rects());
}

#[tokio::test]
async fn presence_is_relayed_with_session_name_but_not_echoed() {
    let base = start(db::connect("sqlite::memory:").await.unwrap()).await;
    let (mut ws_a, _) = join(&base, "q3", "bia").await;
    let (mut ws_b, _) = join(&base, "q3", "ana").await;

    // "bia" tenta se passar por "ana": o servidor usa o nome da sessão.
    let spoofed = r#"{"type":"cursor","user":"ana","x":1.0,"y":2.0}"#;
    ws_a.send(Message::Text(spoofed.into())).await.unwrap();
    let cursor = r#"{"type":"cursor","user":"bia","x":1.0,"y":2.0}"#;

    let got = tokio::time::timeout(Duration::from_secs(5), ws_b.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got, Message::Text(cursor.into()));

    let echoed = tokio::time::timeout(Duration::from_millis(200), ws_a.next()).await;
    assert!(
        echoed.is_err(),
        "o remetente não deve receber o próprio cursor"
    );
}
