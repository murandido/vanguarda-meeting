//! Salas WebSocket: uma por quadro. O servidor guarda a própria cópia Loro
//! do quadro, repassa as atualizações para os outros participantes e salva
//! snapshots no SQLite.
//!
//! Robustez:
//! - **Ping/pong**: o servidor manda um Ping a cada [`RoomTiming::ping_every`]
//!   e derruba a conexão que passar [`RoomTiming::pong_timeout`] sem dar
//!   sinal de vida (proxies e redes móveis matam sockets ociosos em silêncio).
//! - **Salvamento agrupado**: cada atualização só marca a sala como suja; uma
//!   tarefa por sala salva no máximo a cada [`RoomTiming::save_every`]. Há um
//!   salvamento final quando o último participante sai e outro no desligamento
//!   gracioso ([`Rooms::shutdown`]).
//! - **Limites**: mensagens acima de [`MAX_MESSAGE_BYTES`] derrubam só a
//!   conexão que as mandou. Cada atualização é testada antes numa cópia do
//!   documento ([`Board::import_within`]): bytes inválidos ou que estourem os
//!   [`Limits`] (formas, tamanho de id, tamanho do snapshot) são descartados
//!   sem repasse. A sala continua de pé em todos os casos.
//! - **Origem**: o upgrade só é aceito com `Origin` igual a `PUBLIC_URL`
//!   (contra *cross-site WebSocket hijacking* a partir de outros subdomínios,
//!   que contam como "same-site" para o cookie `SameSite=Lax`).

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Extension,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket, close_code},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use canvas::{Board, Limits};
use futures_util::{SinkExt, StreamExt};
use shared::PresenceMsg;
use sqlx::SqlitePool;
use tokio::{
    sync::{Mutex, Notify, broadcast},
    task::JoinHandle,
    time::Instant,
};

use crate::{AppState, auth::CurrentUser, boards, db};

const ROOM_BUFFER: usize = 256;

/// Maior mensagem aceita de um cliente (256 KiB).
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// Tempos das salas. Os testes usam valores curtos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomTiming {
    /// Intervalo entre Pings do servidor.
    pub ping_every: Duration,
    /// Silêncio máximo do cliente (nem Pong, nem mensagem) antes de fechar.
    pub pong_timeout: Duration,
    /// Intervalo mínimo entre dois salvamentos do mesmo quadro.
    pub save_every: Duration,
}

impl Default for RoomTiming {
    fn default() -> Self {
        Self {
            ping_every: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(60),
            save_every: Duration::from_secs(2),
        }
    }
}

/// Sinal de vida de uma conexão: qualquer mensagem recebida (inclusive Pong)
/// adia o prazo.
#[derive(Debug, Clone, Copy)]
struct Liveness {
    last_seen: Instant,
    timeout: Duration,
}

impl Liveness {
    fn new(now: Instant, timeout: Duration) -> Self {
        Self {
            last_seen: now,
            timeout,
        }
    }

    fn saw(&mut self, now: Instant) {
        self.last_seen = self.last_seen.max(now);
    }

    /// Instante a partir do qual a conexão é dada como morta.
    fn deadline(&self) -> Instant {
        self.last_seen + self.timeout
    }

    fn is_dead(&self, now: Instant) -> bool {
        now >= self.deadline()
    }
}

#[derive(Clone)]
enum Frame {
    /// Mensagem de um participante para os demais.
    Relay { from: u64, msg: Message },
    /// O servidor vai desligar: todo mundo fecha.
    Shutdown,
}

struct Doc {
    board: Board,
    /// Há mudanças ainda não salvas no SQLite.
    dirty: bool,
}

struct Room {
    id: String,
    tx: broadcast::Sender<Frame>,
    doc: Mutex<Doc>,
    /// Serializa os salvamentos: um snapshot antigo nunca sobrescreve um novo.
    save_lock: Mutex<()>,
    /// Acorda a tarefa de salvamento.
    changed: Notify,
    /// Conexões na sala; só muda com o mapa de salas travado.
    members: AtomicUsize,
    saver: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl Room {
    /// Salva o snapshot se houver mudanças pendentes.
    async fn flush(&self, db: &SqlitePool) -> anyhow::Result<()> {
        let _saving = self.save_lock.lock().await;
        let snapshot = {
            let mut doc = self.doc.lock().await;
            if !doc.dirty {
                return Ok(());
            }
            let snapshot = doc.board.snapshot()?;
            doc.dirty = false;
            snapshot
        };
        if let Err(e) = db::save_snapshot(db, &self.id, &snapshot).await {
            // Fica sujo para a próxima tentativa.
            self.doc.lock().await.dirty = true;
            return Err(e.into());
        }
        Ok(())
    }

    fn stop_saver(&self) {
        if let Some(task) = self.saver.lock().expect("mutex do saver").take() {
            task.abort();
        }
    }
}

/// Tarefa de salvamento de uma sala: depois de uma mudança, espera
/// `every` (juntando as mudanças que chegarem nesse meio-tempo) e salva.
async fn saver(room: Arc<Room>, db: SqlitePool, every: Duration) {
    loop {
        room.changed.notified().await;
        tokio::time::sleep(every).await;
        if let Err(e) = room.flush(&db).await {
            tracing::error!(board = room.id, "falha ao salvar snapshot: {e:#}");
            room.changed.notify_one();
        }
    }
}

struct Inner {
    rooms: Mutex<HashMap<String, Arc<Room>>>,
    next_conn: AtomicU64,
    timing: RoomTiming,
    closing: AtomicBool,
}

#[derive(Clone)]
pub struct Rooms {
    inner: Arc<Inner>,
}

impl Default for Rooms {
    fn default() -> Self {
        Self::new(RoomTiming::default())
    }
}

impl Rooms {
    #[must_use]
    pub fn new(timing: RoomTiming) -> Self {
        Self {
            inner: Arc::new(Inner {
                rooms: Mutex::new(HashMap::new()),
                next_conn: AtomicU64::new(0),
                timing,
                closing: AtomicBool::new(false),
            }),
        }
    }

    #[must_use]
    pub fn timing(&self) -> RoomTiming {
        self.inner.timing
    }

    fn is_closing(&self) -> bool {
        self.inner.closing.load(Ordering::SeqCst)
    }

    /// Quantas salas estão abertas na memória.
    pub async fn open_rooms(&self) -> usize {
        self.inner.rooms.lock().await.len()
    }

    /// Entra na sala (criando-a se preciso). A assinatura do canal sai daqui,
    /// com o mapa travado: um `leave` concorrente não consegue remover a sala
    /// entre a entrada e a assinatura.
    async fn join(
        &self,
        db: &SqlitePool,
        board_id: &str,
    ) -> anyhow::Result<(Arc<Room>, broadcast::Receiver<Frame>)> {
        anyhow::ensure!(!self.is_closing(), "servidor desligando");
        let mut rooms = self.inner.rooms.lock().await;
        if let Some(room) = rooms.get(board_id) {
            room.members.fetch_add(1, Ordering::SeqCst);
            return Ok((room.clone(), room.tx.subscribe()));
        }
        let board = match db::load_snapshot(db, board_id).await? {
            Some(bytes) => Board::from_snapshot(&bytes)?,
            None => Board::new(),
        };
        let room = Arc::new(Room {
            id: board_id.to_owned(),
            tx: broadcast::channel(ROOM_BUFFER).0,
            doc: Mutex::new(Doc {
                board,
                dirty: false,
            }),
            save_lock: Mutex::new(()),
            changed: Notify::new(),
            members: AtomicUsize::new(1),
            saver: std::sync::Mutex::new(None),
        });
        let task = tokio::spawn(saver(
            room.clone(),
            db.clone(),
            self.inner.timing.save_every,
        ));
        *room.saver.lock().expect("mutex do saver") = Some(task);
        rooms.insert(board_id.to_owned(), room.clone());
        let rx = room.tx.subscribe();
        Ok((room, rx))
    }

    /// Saída de um participante. O último a sair salva o quadro e libera a
    /// sala da memória.
    async fn leave(&self, db: &SqlitePool, room: &Arc<Room>) {
        // O mapa fica travado durante o salvamento final: assim ninguém
        // recria a sala a partir de um snapshot velho do banco.
        let mut rooms = self.inner.rooms.lock().await;
        if room.members.fetch_sub(1, Ordering::SeqCst) != 1 {
            return;
        }
        if let Err(e) = room.flush(db).await {
            // A sala continua na memória; a tarefa de salvamento tenta de novo.
            tracing::error!(board = room.id, "falha no salvamento final: {e:#}");
            room.changed.notify_one();
            return;
        }
        if rooms.get(&room.id).is_some_and(|r| Arc::ptr_eq(r, room)) {
            rooms.remove(&room.id);
        }
        room.stop_saver();
    }

    /// Desligamento gracioso: recusa novas atualizações, fecha as conexões e
    /// salva todos os quadros com mudanças pendentes.
    pub async fn shutdown(&self, db: &SqlitePool) {
        self.inner.closing.store(true, Ordering::SeqCst);
        let rooms: Vec<Arc<Room>> = self.inner.rooms.lock().await.values().cloned().collect();
        for room in rooms {
            let _ = room.tx.send(Frame::Shutdown);
            match room.flush(db).await {
                Ok(()) => room.stop_saver(),
                Err(e) => {
                    tracing::error!(board = room.id, "falha ao salvar no desligamento: {e:#}");
                }
            }
        }
    }
}

/// `Origin` da requisição bate exatamente com a URL pública do app?
fn same_origin(headers: &HeaderMap, public_url: &str) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|origin| origin.eq_ignore_ascii_case(public_url))
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Extension(user): Extension<CurrentUser>,
    Path(board): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if !same_origin(&headers, &state.auth.public_url) {
        tracing::warn!(board, origin = ?headers.get(header::ORIGIN), "WebSocket recusado: origem");
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Err(status) = boards::open_board(&state.db, &board).await {
        return status.into_response();
    }
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let (room, rx) = match state.rooms.join(&state.db, &board).await {
                Ok(joined) => joined,
                Err(e) => {
                    tracing::warn!(board, "não foi possível entrar na sala: {e:#}");
                    return;
                }
            };
            if let Err(e) = handle_socket(socket, &state.rooms, &room, rx, user.login).await {
                tracing::warn!(board, "conexão encerrada com erro: {e:#}");
            }
            state.rooms.leave(&state.db, &room).await;
        })
}

fn close(code: u16, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }))
}

async fn handle_socket(
    socket: WebSocket,
    rooms: &Rooms,
    room: &Room,
    mut rx: broadcast::Receiver<Frame>,
    login: String,
) -> anyhow::Result<()> {
    let me = rooms.inner.next_conn.fetch_add(1, Ordering::Relaxed);
    let timing = rooms.inner.timing;

    // `rx` foi assinado antes do snapshot: nada que chegue no meio se perde
    // (receber algo repetido é inofensivo num CRDT).
    let snapshot = room.doc.lock().await.board.snapshot()?;

    let (mut sink, mut stream) = socket.split();
    sink.send(Message::Binary(snapshot.into())).await?;

    let mut ping = tokio::time::interval_at(Instant::now() + timing.ping_every, timing.ping_every);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut alive = Liveness::new(Instant::now(), timing.pong_timeout);
    let mut sent_cursor = false;
    // Depois de uma falha de envio (ou de pedirmos para fechar) não mandamos
    // mais nada, mas continuamos lendo até o fim: atualizações que o cliente
    // mandou antes de sair ainda estão no buffer e não podem se perder.
    let mut can_send = true;

    let result = loop {
        tokio::select! {
            msg = stream.next() => {
                let msg = match msg {
                    None => break Ok(()),
                    Some(Err(e)) => break Err(e.into()),
                    Some(Ok(msg)) => msg,
                };
                alive.saw(Instant::now());
                match msg {
                    Message::Binary(bytes) => {
                        {
                            let mut doc = room.doc.lock().await;
                            // Conferido com o documento travado: o salvamento
                            // do desligamento já inclui tudo o que passou daqui.
                            if rooms.is_closing() {
                                break Ok(());
                            }
                            if let Err(e) = doc.board.import_within(&bytes, &Limits::default()) {
                                tracing::warn!(board = room.id, "atualização rejeitada: {e}");
                                continue;
                            }
                            doc.dirty = true;
                        }
                        room.changed.notify_one();
                        let _ = room.tx.send(Frame::Relay {
                            from: me,
                            msg: Message::Binary(bytes),
                        });
                    }
                    Message::Text(text) => {
                        // Só cursores; o nome vem da sessão, nunca do cliente.
                        let Ok(PresenceMsg::Cursor { x, y, .. }) = serde_json::from_str(&text)
                        else {
                            continue;
                        };
                        let cursor = PresenceMsg::Cursor {
                            user: login.clone(),
                            x,
                            y,
                        };
                        sent_cursor = true;
                        let _ = room.tx.send(Frame::Relay {
                            from: me,
                            msg: Message::Text(serde_json::to_string(&cursor)?.into()),
                        });
                    }
                    Message::Close(_) => break Ok(()),
                    Message::Ping(_) | Message::Pong(_) => {}
                }
            }
            frame = rx.recv() => match frame {
                Ok(Frame::Relay { from, msg }) => {
                    if can_send && from != me && sink.send(msg).await.is_err() {
                        can_send = false;
                    }
                }
                Ok(Frame::Shutdown) => {
                    let _ = sink.send(close(close_code::RESTART, "servidor reiniciando")).await;
                    break Ok(());
                }
                // Um cliente lento perdeu mensagens: pede para ele reconectar
                // e receber um snapshot novo (lendo o que ele ainda mandar).
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if can_send {
                        let _ = sink.send(close(close_code::AGAIN, "atrasado; reconecte")).await;
                        can_send = false;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break Ok(()),
            },
            _ = ping.tick() => {
                if can_send && sink.send(Message::Ping(axum::body::Bytes::new())).await.is_err() {
                    can_send = false;
                }
            }
            () = tokio::time::sleep_until(alive.deadline()) => {
                if alive.is_dead(Instant::now()) {
                    tracing::info!(board = room.id, login, "sem resposta ao ping; fechando");
                    let _ = sink.send(close(close_code::AWAY, "sem resposta")).await;
                    break Ok(());
                }
            }
        }
    };

    if sent_cursor {
        let leave = serde_json::to_string(&PresenceMsg::Leave { user: login })?;
        let _ = room.tx.send(Frame::Relay {
            from: me,
            msg: Message::Text(leave.into()),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_timing() {
        let t = RoomTiming::default();
        assert_eq!(t.ping_every, Duration::from_secs(30));
        assert_eq!(t.pong_timeout, Duration::from_secs(60));
        assert_eq!(t.save_every, Duration::from_secs(2));
    }

    #[test]
    fn liveness_deadline_moves_with_activity() {
        let start = Instant::now();
        let mut alive = Liveness::new(start, Duration::from_secs(60));
        assert_eq!(alive.deadline(), start + Duration::from_secs(60));
        assert!(!alive.is_dead(start + Duration::from_secs(59)));
        assert!(alive.is_dead(start + Duration::from_secs(60)));

        // Um Pong aos 30 s empurra o prazo para 90 s.
        alive.saw(start + Duration::from_secs(30));
        assert!(!alive.is_dead(start + Duration::from_secs(89)));
        assert!(alive.is_dead(start + Duration::from_secs(90)));

        // Relógio fora de ordem não puxa o prazo para trás.
        alive.saw(start + Duration::from_secs(10));
        assert_eq!(alive.deadline(), start + Duration::from_secs(90));
    }

    #[test]
    fn origin_must_match_public_url() {
        let url = "https://vanguarda.exemplo";
        let with = |origin: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::ORIGIN, origin.parse().unwrap());
            h
        };
        assert!(same_origin(&with("https://vanguarda.exemplo"), url));
        assert!(same_origin(&with("https://Vanguarda.Exemplo"), url));
        assert!(!same_origin(&with("https://m80.exemplo"), url));
        assert!(!same_origin(&with("http://vanguarda.exemplo"), url));
        assert!(!same_origin(&with("https://vanguarda.exemplo:8443"), url));
        assert!(!same_origin(&with("null"), url));
        assert!(!same_origin(&HeaderMap::new(), url));
    }
}
