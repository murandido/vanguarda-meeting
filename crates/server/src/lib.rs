//! Servidor do Vanguarda: páginas HTMX, relay WebSocket dos quadros e SQLite.

pub mod auth;
mod boards;
pub mod db;
mod pages;
pub mod ritmo;
mod rooms;
mod startup;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use axum::{
    Router, middleware,
    routing::{get, post},
};
use sqlx::SqlitePool;
use tower_http::services::ServeDir;

pub use auth::AuthConfig;
pub use rooms::{MAX_MESSAGE_BYTES, RoomTiming, Rooms};
pub use startup::{Config, serve};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub rooms: Rooms,
    pub auth: Arc<AuthConfig>,
    pub http: reqwest::Client,
    pub clock: Clock,
    pub ritmo: ritmo::Events,
}

impl AppState {
    #[must_use]
    pub fn new(db: SqlitePool, auth: AuthConfig) -> Self {
        Self {
            db,
            rooms: Rooms::default(),
            auth: Arc::new(auth),
            http: reqwest::Client::new(),
            clock: Clock::default(),
            ritmo: ritmo::Events::default(),
        }
    }

    /// Troca as salas (os testes usam tempos curtos de ping e salvamento).
    #[must_use]
    pub fn with_rooms(mut self, rooms: Rooms) -> Self {
        self.rooms = rooms;
        self
    }
}

/// Relógio do servidor em segundos Unix. Os handlers leem o "agora" daqui,
/// e não do sistema, para os testes fixarem datas.
#[derive(Clone, Default)]
pub struct Clock(Option<Arc<AtomicI64>>);

impl Clock {
    /// Relógio parado em `secs`, ajustável com [`Clock::set`].
    #[must_use]
    pub fn fixed(secs: i64) -> Self {
        Self(Some(Arc::new(AtomicI64::new(secs))))
    }

    /// Muda a hora de um relógio fixo (no relógio do sistema, não faz nada).
    pub fn set(&self, secs: i64) {
        if let Some(t) = &self.0 {
            t.store(secs, Ordering::Relaxed);
        }
    }

    #[must_use]
    pub fn now(&self) -> i64 {
        self.0
            .as_ref()
            .map_or_else(auth::now, |t| t.load(Ordering::Relaxed))
    }
}

/// How often expired and revoked sessions are cleaned up.
pub const SESSION_CLEANUP_EVERY: std::time::Duration = std::time::Duration::from_hours(1);

/// Cleans up sessions now (at boot, so removed guests lose access) and then
/// every `every`.
#[must_use = "dropping the handle detaches the task"]
pub fn spawn_session_cleanup(
    state: AppState,
    every: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            match auth::prune_sessions(&state).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("{n} sessões expiradas ou revogadas apagadas"),
                Err(e) => tracing::error!("falha ao limpar sessões: {e}"),
            }
        }
    })
}

/// Onde ficam os arquivos estáticos: `static/` do crate e o `pkg/` gerado
/// pelo `wasm-pack`.
#[derive(Clone)]
pub struct Assets {
    pub static_dir: PathBuf,
    pub pkg_dir: PathBuf,
}

impl Default for Assets {
    fn default() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Self {
            static_dir: root.join("static"),
            pkg_dir: root.join("../canvas/pkg"),
        }
    }
}

pub fn app(state: AppState, assets: &Assets) -> Router {
    Router::new()
        .route("/", get(pages::index))
        .route("/b/{board}", get(pages::board))
        .route("/health", get(|| async { "ok" }))
        .route("/ws/{board}", get(rooms::ws_handler))
        // Cartazes: lista de quadros (CA-09).
        .route("/cartazes", get(boards::list).post(boards::create))
        .route("/cartazes/{board}/renomear", post(boards::rename))
        .route("/cartazes/{board}/arquivar", post(boards::archive))
        .route("/cartazes/{board}/desarquivar", post(boards::unarchive))
        .route("/login", get(auth::login_page))
        .route("/logout", post(auth::logout))
        .route("/auth/github", get(auth::github_start))
        .route("/auth/github/callback", get(auth::github_callback))
        .route("/auth/dev/{login}", get(auth::dev_login))
        .merge(ritmo::routes())
        .nest_service("/static", ServeDir::new(&assets.static_dir))
        .nest_service("/pkg", ServeDir::new(&assets.pkg_dir))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_user,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::check_origin,
        ))
        .with_state(state)
}

/// Porta local do servidor a partir do `BIND` (padrão 3000).
#[must_use]
pub fn health_port(bind: Option<&str>) -> u16 {
    bind.and_then(|b| b.parse::<std::net::SocketAddr>().ok())
        .map_or(3000, |addr| addr.port())
}

/// Faz `GET /health` em `127.0.0.1:{port}` e diz se respondeu `200 ok`.
pub async fn healthcheck(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let check = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok::<_, std::io::Error>(response)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(3), check).await {
        Ok(Ok(response)) => response.starts_with(b"HTTP/1.1 200") && response.ends_with(b"ok"),
        _ => false,
    }
}
