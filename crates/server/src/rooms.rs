//! Salas WebSocket: uma por quadro. O servidor guarda a própria cópia Loro
//! do quadro, repassa as atualizações para os outros participantes e salva
//! snapshots no SQLite.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use axum::{
    Extension,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use canvas::Board;
use futures_util::{SinkExt, StreamExt};
use shared::PresenceMsg;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, broadcast};

use crate::{AppState, auth::CurrentUser, db};

const ROOM_BUFFER: usize = 256;

#[derive(Clone)]
struct Frame {
    from: u64,
    msg: Message,
}

struct Room {
    tx: broadcast::Sender<Frame>,
    board: Mutex<Board>,
}

#[derive(Clone, Default)]
pub struct Rooms {
    rooms: Arc<Mutex<HashMap<String, Arc<Room>>>>,
    next_conn: Arc<AtomicU64>,
}

impl Rooms {
    async fn join(&self, db: &SqlitePool, board_id: &str) -> anyhow::Result<Arc<Room>> {
        let mut rooms = self.rooms.lock().await;
        if let Some(room) = rooms.get(board_id) {
            return Ok(room.clone());
        }
        let board = match db::load_snapshot(db, board_id).await? {
            Some(bytes) => Board::from_snapshot(&bytes)?,
            None => Board::new(),
        };
        let room = Arc::new(Room {
            tx: broadcast::channel(ROOM_BUFFER).0,
            board: Mutex::new(board),
        });
        rooms.insert(board_id.to_owned(), room.clone());
        Ok(room)
    }

    /// Descarta a sala da memória quando o último participante sai; o estado
    /// já está no SQLite.
    async fn leave(&self, board_id: &str) {
        let mut rooms = self.rooms.lock().await;
        if rooms
            .get(board_id)
            .is_some_and(|room| room.tx.receiver_count() == 0)
        {
            rooms.remove(board_id);
        }
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Extension(user): Extension<CurrentUser>,
    Path(board): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if !shared::is_valid_board_id(&board) {
        return StatusCode::NOT_FOUND.into_response();
    }
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = handle_socket(socket, &state, &board, user.login).await {
            tracing::warn!(board, "conexão encerrada com erro: {e:#}");
        }
        state.rooms.leave(&board).await;
    })
}

async fn handle_socket(
    socket: WebSocket,
    state: &AppState,
    board_id: &str,
    login: String,
) -> anyhow::Result<()> {
    let room = state.rooms.join(&state.db, board_id).await?;
    let me = state.rooms.next_conn.fetch_add(1, Ordering::Relaxed);

    // Assinar antes de tirar o snapshot: nada que chegue no meio se perde
    // (receber algo repetido é inofensivo num CRDT).
    let mut rx = room.tx.subscribe();
    let snapshot = room.board.lock().await.snapshot()?;

    let (mut sink, mut stream) = socket.split();
    sink.send(Message::Binary(snapshot.into())).await?;

    let mut forward = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(frame) if frame.from != me => {
                    if sink.send(frame.msg).await.is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                // Um cliente lento perdeu mensagens: fecha para ele reconectar
                // e receber um snapshot novo.
                Err(
                    broadcast::error::RecvError::Lagged(_) | broadcast::error::RecvError::Closed,
                ) => {
                    break;
                }
            }
        }
    });

    let mut sent_cursor = false;
    let result = loop {
        let msg = tokio::select! {
            msg = stream.next() => msg,
            _ = &mut forward => break Ok(()),
        };
        let Some(msg) = msg else { break Ok(()) };
        match msg {
            Ok(Message::Binary(bytes)) => {
                let snapshot = {
                    let mut board = room.board.lock().await;
                    if let Err(e) = board.import(&bytes) {
                        tracing::warn!(board_id, "atualização rejeitada: {e}");
                        continue;
                    }
                    board.snapshot()?
                };
                let _ = room.tx.send(Frame {
                    from: me,
                    msg: Message::Binary(bytes),
                });
                // Fase 0: salva a cada atualização. Agrupar em lotes quando o
                // volume justificar.
                db::save_snapshot(&state.db, board_id, &snapshot).await?;
            }
            Ok(Message::Text(text)) => {
                // Só cursores; o nome vem da sessão, nunca do cliente.
                let Ok(PresenceMsg::Cursor { x, y, .. }) = serde_json::from_str(&text) else {
                    continue;
                };
                let cursor = PresenceMsg::Cursor {
                    user: login.clone(),
                    x,
                    y,
                };
                sent_cursor = true;
                let _ = room.tx.send(Frame {
                    from: me,
                    msg: Message::Text(serde_json::to_string(&cursor)?.into()),
                });
            }
            Ok(Message::Close(_)) => break Ok(()),
            Ok(_) => {}
            Err(e) => break Err(e.into()),
        }
    };

    forward.abort();
    if sent_cursor {
        let leave = serde_json::to_string(&PresenceMsg::Leave { user: login })?;
        let _ = room.tx.send(Frame {
            from: me,
            msg: Message::Text(leave.into()),
        });
    }
    result
}
