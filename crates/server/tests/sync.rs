//! Sincronização ponta a ponta pelo servidor real: clientes WebSocket com o
//! mesmo motor `canvas` que roda no navegador.

use std::time::Duration;

use canvas::Board;
use futures_util::{SinkExt, StreamExt};
use server::{AppState, Assets, MAX_MESSAGE_BYTES, RoomTiming, Rooms, app, db};
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        self, Message,
        client::IntoClientRequest,
        http::{StatusCode, header},
        protocol::frame::coding::CloseCode,
    },
};

mod common;

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// `PUBLIC_URL` padrão da configuração de testes: o `Origin` esperado.
const ORIGIN: &str = "http://127.0.0.1:3000";

struct Server {
    base: String,
    pool: SqlitePool,
    rooms: Rooms,
}

/// Tempos longos: nada de ping nem salvamento periódico durante o teste.
fn slow() -> RoomTiming {
    RoomTiming {
        ping_every: Duration::from_secs(3_600),
        pong_timeout: Duration::from_secs(7_200),
        save_every: Duration::from_secs(3_600),
    }
}

async fn start_with(pool: SqlitePool, timing: RoomTiming) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let rooms = Rooms::new(timing);
    let state = AppState::new(pool.clone(), common::dev_auth()).with_rooms(rooms.clone());
    let router = app(state, &Assets::default());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Server {
        base: format!("ws://{addr}/ws"),
        pool,
        rooms,
    }
}

async fn start(pool: SqlitePool) -> Server {
    start_with(pool, RoomTiming::default()).await
}

async fn memory() -> SqlitePool {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    for id in ["q1", "q2", "q3", "q4", "q5", "q6", "q7", "q8", "q9"] {
        common::create_board(&pool, id).await;
    }
    pool
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

async fn connect(
    server: &Server,
    board: &str,
    login: &str,
    origin: Option<&str>,
) -> Result<Ws, tungstenite::Error> {
    let mut req = format!("{}/{board}", server.base)
        .into_client_request()
        .unwrap();
    let cookie = session(&server.pool, login).await;
    req.headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    if let Some(origin) = origin {
        req.headers_mut()
            .insert(header::ORIGIN, origin.parse().unwrap());
    }
    connect_async(req).await.map(|(ws, _)| ws)
}

fn refused_with(result: Result<Ws, tungstenite::Error>) -> StatusCode {
    match result {
        Err(tungstenite::Error::Http(res)) => res.status(),
        Err(e) => panic!("erro inesperado: {e}"),
        Ok(_) => panic!("conexão deveria ter sido recusada"),
    }
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

/// Lê até o servidor fechar; devolve o código do Close, se veio um.
async fn closed(ws: &mut Ws) -> Option<CloseCode> {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("fechamento em até 5 s");
        match msg {
            Some(Ok(Message::Close(frame))) => return frame.map(|f| f.code),
            Some(Ok(_)) => {}
            None | Some(Err(_)) => return None,
        }
    }
}

async fn join(server: &Server, board: &str, login: &str) -> (Ws, Board) {
    let mut ws = connect(server, board, login, Some(ORIGIN)).await.unwrap();
    let snapshot = next_binary(&mut ws).await;
    let mut local = Board::new();
    local.import(&snapshot).unwrap();
    (ws, local)
}

async fn push(ws: &mut Ws, board: &mut Board) {
    let update = board.take_pending_update().unwrap().unwrap();
    ws.send(Message::Binary(update.into())).await.unwrap();
}

/// Espera até o snapshot salvo no banco satisfazer `ok`.
async fn wait_saved(pool: &SqlitePool, board: &str, ok: impl Fn(&Board) -> bool) -> Board {
    for _ in 0..100 {
        if let Some(bytes) = db::load_snapshot(pool, board).await.unwrap() {
            let saved = Board::from_snapshot(&bytes).unwrap();
            if ok(&saved) {
                return saved;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let last = db::load_snapshot(pool, board)
        .await
        .unwrap()
        .map(|b| Board::from_snapshot(&b).unwrap().shapes().len());
    panic!("snapshot de {board} não foi salvo a tempo (formas: {last:?})");
}

async fn wait_rooms(rooms: &Rooms, expected: usize) {
    for _ in 0..100 {
        if rooms.open_rooms().await == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("esperava {expected} salas, há {}", rooms.open_rooms().await);
}

#[tokio::test]
async fn edits_reach_the_other_client() {
    let base = start(memory().await).await;
    let (mut ws_a, mut a) = join(&base, "q1", "ana").await;
    let (mut ws_b, mut b) = join(&base, "q1", "bia").await;

    a.add_rect("r1", 10.0, 20.0, 100.0, 50.0);
    push(&mut ws_a, &mut a).await;
    b.import(&next_binary(&mut ws_b).await).unwrap();
    assert_eq!(b.shapes(), a.shapes());

    b.move_shape("r1", 300.0, 400.0).unwrap();
    push(&mut ws_b, &mut b).await;
    a.import(&next_binary(&mut ws_a).await).unwrap();
    assert_eq!(a.shapes()[0].x, 300.0);
}

#[tokio::test]
async fn late_joiner_and_restart_get_the_saved_board() {
    let pool = memory().await;
    let base = start(pool.clone()).await;

    let (mut ws_a, mut a) = join(&base, "q2", "ana").await;
    a.add_rect("r1", 1.0, 2.0, 3.0, 4.0);
    push(&mut ws_a, &mut a).await;

    // Quem chega depois recebe o estado no snapshot inicial.
    let (ws_b, b) = join(&base, "q2", "bia").await;
    assert_eq!(b.shapes(), a.shapes());

    // Todos saem: o último salva. Um servidor novo sobre o mesmo banco
    // recebe o quadro.
    drop((ws_a, ws_b));
    wait_saved(&pool, "q2", |s| s.shapes() == a.shapes()).await;
    let base2 = start(pool).await;
    let (_ws_c, c) = join(&base2, "q2", "ana").await;
    assert_eq!(c.shapes(), a.shapes());
}

#[tokio::test]
async fn presence_is_relayed_with_session_name_but_not_echoed() {
    let base = start(memory().await).await;
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

    // Ao sair, os outros recebem o "leave".
    drop(ws_a);
    let got = tokio::time::timeout(Duration::from_secs(5), ws_b.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        got,
        Message::Text(r#"{"type":"leave","user":"bia"}"#.into())
    );
}

#[tokio::test]
async fn unknown_archived_and_foreign_origin_are_refused() {
    let base = start(memory().await).await;
    let status = refused_with(connect(&base, "nao-existe", "ana", Some(ORIGIN)).await);
    assert_eq!(status, StatusCode::NOT_FOUND);
    let status = refused_with(connect(&base, "Ruim", "ana", Some(ORIGIN)).await);
    assert_eq!(status, StatusCode::NOT_FOUND);

    db::set_board_archived(&base.pool, "q4", true)
        .await
        .unwrap();
    let status = refused_with(connect(&base, "q4", "ana", Some(ORIGIN)).await);
    assert_eq!(status, StatusCode::GONE);

    // CSWSH: outro subdomínio (mesmo "site" para o cookie) ou sem Origin.
    for origin in [Some("https://sibling.example.com"), None] {
        let status = refused_with(connect(&base, "q5", "ana", origin).await);
        assert_eq!(status, StatusCode::FORBIDDEN, "{origin:?}");
    }
    assert!(connect(&base, "q5", "ana", Some(ORIGIN)).await.is_ok());
}

#[tokio::test]
async fn saves_are_debounced_and_flushed_when_the_last_one_leaves() {
    let pool = memory().await;
    let base = start_with(pool.clone(), slow()).await;
    let (mut ws_a, mut a) = join(&base, "q6", "ana").await;
    let (mut ws_b, mut b) = join(&base, "q6", "bia").await;

    for i in 0..20 {
        a.add_rect(&format!("r{i}"), f64::from(i), 0.0, 1.0, 1.0);
        push(&mut ws_a, &mut a).await;
    }
    while b.shapes().len() < 20 {
        b.import(&next_binary(&mut ws_b).await).unwrap();
    }
    // O servidor já aplicou tudo, mas o salvamento periódico ainda não veio.
    assert_eq!(db::load_snapshot(&pool, "q6").await.unwrap(), None);

    // Um sai: ainda há gente, nada salvo. O último sai: salva na hora.
    drop(ws_b);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(db::load_snapshot(&pool, "q6").await.unwrap(), None);
    drop(ws_a);
    let saved = wait_saved(&pool, "q6", |s| s.shapes().len() == 20).await;
    assert_eq!(saved.shapes(), a.shapes());
    wait_rooms(&base.rooms, 0).await;
}

#[tokio::test]
async fn periodic_save_after_quiet_period() {
    let pool = memory().await;
    let timing = RoomTiming {
        save_every: Duration::from_millis(100),
        ..slow()
    };
    let base = start_with(pool.clone(), timing).await;
    let (mut ws_a, mut a) = join(&base, "q7", "ana").await;
    for i in 0..10 {
        a.add_rect(&format!("r{i}"), 0.0, 0.0, 1.0, 1.0);
        push(&mut ws_a, &mut a).await;
    }
    // Ainda conectado: o salvamento periódico grava tudo.
    let saved = wait_saved(&pool, "q7", |s| s.shapes().len() == 10).await;
    assert_eq!(saved.shapes(), a.shapes());
}

#[tokio::test]
async fn graceful_shutdown_saves_and_closes() {
    let pool = memory().await;
    let base = start_with(pool.clone(), slow()).await;
    let (mut ws_a, mut a) = join(&base, "q8", "ana").await;
    let (mut ws_b, mut b) = join(&base, "q8", "bia").await;
    a.add_rect("r1", 1.0, 1.0, 1.0, 1.0);
    push(&mut ws_a, &mut a).await;
    b.import(&next_binary(&mut ws_b).await).unwrap();

    base.rooms.shutdown(&pool).await;
    let bytes = db::load_snapshot(&pool, "q8").await.unwrap().unwrap();
    assert_eq!(Board::from_snapshot(&bytes).unwrap().shapes(), a.shapes());
    assert_eq!(closed(&mut ws_a).await, Some(CloseCode::Restart));
    assert_eq!(closed(&mut ws_b).await, Some(CloseCode::Restart));

    // Depois do desligamento ninguém entra.
    let mut ws_c = connect(&base, "q8", "ana", Some(ORIGIN)).await.unwrap();
    assert_eq!(closed(&mut ws_c).await, None);
}

#[tokio::test]
async fn server_pings_and_drops_silent_clients() {
    let timing = RoomTiming {
        ping_every: Duration::from_millis(50),
        pong_timeout: Duration::from_millis(300),
        ..slow()
    };
    let base = start_with(memory().await, timing).await;

    // Quem lê responde os pings sozinho (tungstenite) e continua vivo; quem
    // some (não lê, não responde) é derrubado.
    let (mut alive, _) = join(&base, "q9", "ana").await;
    let (mut silent, _) = join(&base, "q9", "bia").await;
    let mut pings = 0;
    let until = tokio::time::Instant::now() + Duration::from_millis(800);
    while let Ok(msg) = tokio::time::timeout_at(until, alive.next()).await {
        match msg.unwrap().unwrap() {
            Message::Ping(_) => pings += 1,
            other => panic!("mensagem inesperada: {other:?}"),
        }
    }
    assert!(pings >= 3, "pings: {pings}");

    // O servidor manda Close (1001) e encerra; o código pode não chegar
    // ao cliente se o TCP já tiver caído, mas a conexão termina.
    closed(&mut silent).await;

    // O cliente vivo segue funcionando na mesma sala.
    alive.send(Message::Text("{}".into())).await.unwrap();
    assert_eq!(base.rooms.open_rooms().await, 1);
}

#[tokio::test]
async fn garbage_and_oversized_frames_do_not_kill_the_room() {
    let base = start(memory().await).await;
    let (mut ws_a, mut a) = join(&base, "q1", "ana").await;
    let (mut ws_b, mut b) = join(&base, "q1", "bia").await;

    // Lixo: ignorado, a conexão continua e nada é repassado.
    ws_a.send(Message::Binary(vec![0xff; 100].into()))
        .await
        .unwrap();
    // Forma com id longo demais: recusada pelos limites do quadro.
    a.add_rect(&"x".repeat(65), 0.0, 0.0, 1.0, 1.0);
    push(&mut ws_a, &mut a).await;

    let mut ok = Board::new();
    ok.add_rect("r1", 1.0, 2.0, 3.0, 4.0);
    push(&mut ws_a, &mut ok).await;
    b.import(&next_binary(&mut ws_b).await).unwrap();
    assert_eq!(b.shapes(), ok.shapes());

    // Grande demais: só a conexão de quem mandou cai.
    let _ = ws_a
        .send(Message::Binary(vec![0u8; MAX_MESSAGE_BYTES + 1].into()))
        .await;
    closed(&mut ws_a).await;

    b.add_rect("r2", 5.0, 5.0, 5.0, 5.0);
    push(&mut ws_b, &mut b).await;
    let (mut ws_c, mut c) = join(&base, "q1", "ana").await;
    while c.shapes() != b.shapes() {
        c.import(&next_binary(&mut ws_c).await).unwrap();
    }
}

#[tokio::test]
async fn rooms_are_freed_and_never_duplicated() {
    let pool = memory().await;
    let base = start(pool.clone()).await;
    let (ws, _) = join(&base, "q2", "ana").await;
    assert_eq!(base.rooms.open_rooms().await, 1);
    drop(ws);
    wait_rooms(&base.rooms, 0).await;

    // Muitas entradas e saídas concorrentes, cada uma criando uma forma.
    // Se duas salas existissem para o mesmo quadro, uma sobrescreveria o
    // snapshot da outra e formas sumiriam.
    let base = std::sync::Arc::new(base);
    let tasks: Vec<_> = (0..20)
        .map(|i| {
            let base = base.clone();
            tokio::spawn(async move {
                let login = if i % 2 == 0 { "ana" } else { "bia" };
                let (mut ws, mut local) = join(&base, "q2", login).await;
                local.add_rect(&format!("r{i}"), 0.0, 0.0, 1.0, 1.0);
                push(&mut ws, &mut local).await;
                ws.close(None).await.unwrap();
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }
    wait_rooms(&base.rooms, 0).await;
    let saved = wait_saved(&pool, "q2", |s| s.shapes().len() == 20).await;
    assert_eq!(saved.shapes().len(), 20);
}
