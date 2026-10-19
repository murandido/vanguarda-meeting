//! Servidor do Vanguarda: páginas HTMX, relay WebSocket dos quadros e SQLite.

pub mod db;
mod pages;
mod rooms;

use std::path::PathBuf;

use axum::{Router, routing::get};
use sqlx::SqlitePool;
use tower_http::services::ServeDir;

pub use rooms::Rooms;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub rooms: Rooms,
}

impl AppState {
    #[must_use]
    pub fn new(db: SqlitePool) -> Self {
        Self {
            db,
            rooms: Rooms::default(),
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
        .nest_service("/static", ServeDir::new(&assets.static_dir))
        .nest_service("/pkg", ServeDir::new(&assets.pkg_dir))
        .with_state(state)
}
