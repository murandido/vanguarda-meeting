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
pub struct Clock {
    fixed: Option<Arc<AtomicI64>>,
    /// Added to the system time (development and E2E only).
    shift: i64,
}

impl Clock {
    /// Relógio parado em `secs`, ajustável com [`Clock::set`].
    #[must_use]
    pub fn fixed(secs: i64) -> Self {
        Self {
            fixed: Some(Arc::new(AtomicI64::new(secs))),
            shift: 0,
        }
    }

    /// System clock moved by `secs`: time keeps running, but on another day.
    /// Used by `DEV_TODAY` so the E2E tests run on a known weekday.
    #[must_use]
    pub fn shifted(secs: i64) -> Self {
        Self {
            fixed: None,
            shift: secs,
        }
    }

    /// Muda a hora de um relógio fixo (no relógio do sistema, não faz nada).
    pub fn set(&self, secs: i64) {
        if let Some(t) = &self.fixed {
            t.store(secs, Ordering::Relaxed);
        }
    }

    #[must_use]
    pub fn now(&self) -> i64 {
        self.fixed
            .as_ref()
            .map_or_else(|| auth::now() + self.shift, |t| t.load(Ordering::Relaxed))
    }
}

/// A running clock whose local date (at `offset_minutes` from UTC) is `day`
/// (`YYYY-MM-DD`), keeping the time of day; `None` if `day` is not a date.
/// `now` is the real current time.
#[must_use]
pub fn clock_on(day: &str, offset_minutes: i64, now: i64) -> Option<Clock> {
    let target = ritmo::date::Date::parse(day)?;
    let today = ritmo::date::Date::from_unix(now, offset_minutes);
    Some(Clock::shifted(target.days_since(today) * 86_400))
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
