//! Servidor do Vanguarda: páginas HTMX, relay WebSocket dos quadros e SQLite.

pub mod auth;
pub mod db;
mod pages;
mod rooms;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router, middleware,
    routing::{get, post},
};
use sqlx::SqlitePool;
use tower_http::services::ServeDir;

pub use auth::AuthConfig;
pub use rooms::Rooms;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub rooms: Rooms,
    pub auth: Arc<AuthConfig>,
    pub http: reqwest::Client,
}

impl AppState {
    #[must_use]
    pub fn new(db: SqlitePool, auth: AuthConfig) -> Self {
        Self {
            db,
            rooms: Rooms::default(),
            auth: Arc::new(auth),
            http: reqwest::Client::new(),
        }
    }
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
        .route("/login", get(auth::login_page))
        .route("/logout", post(auth::logout))
        .route("/auth/github", get(auth::github_start))
        .route("/auth/github/callback", get(auth::github_callback))
        .route("/auth/dev/{login}", get(auth::dev_login))
        .nest_service("/static", ServeDir::new(&assets.static_dir))
        .nest_service("/pkg", ServeDir::new(&assets.pkg_dir))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_user,
        ))
        .with_state(state)
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
