//! Server configuration from the environment and the serve loop with
//! graceful shutdown. Kept out of `main.rs` so it can be tested end to end.

use std::{future::Future, net::SocketAddr};

use tokio::net::TcpListener;

use crate::{AppState, Assets, AuthConfig, app, db};

/// Everything `main` needs to start the server.
pub struct Config {
    pub db_url: String,
    pub addr: SocketAddr,
    pub assets: Assets,
    pub auth: AuthConfig,
    /// E2E only (`DEV_TODAY`, requires `DEV_LOGIN`): pretend today is this
    /// `YYYY-MM-DD` (Brasília time), so the Ritmo tests run on a known weekday.
    pub dev_today: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    /// Reads the configuration from a variable source (testable without
    /// touching the process environment).
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let db_url = var("DATABASE_URL").unwrap_or_else(|| "sqlite://vanguarda.db".to_owned());
        let addr: SocketAddr = var("BIND")
            .unwrap_or_else(|| "127.0.0.1:3000".to_owned())
            .parse()?;

        let mut assets = Assets::default();
        if let Some(dir) = var("STATIC_DIR") {
            assets.static_dir = dir.into();
        }
        if let Some(dir) = var("PKG_DIR") {
            assets.pkg_dir = dir.into();
        }

        let auth = AuthConfig::from_vars(&var)?;
        anyhow::ensure!(
            !auth.dev_login || addr.ip().is_loopback(),
            "DEV_LOGIN só é permitido com BIND em loopback (127.0.0.1)"
        );

        let dev_today = var("DEV_TODAY");
        anyhow::ensure!(
            dev_today.is_none() || auth.dev_login,
            "DEV_TODAY só é permitido junto com DEV_LOGIN"
        );

        Ok(Self {
            db_url,
            addr,
            assets,
            auth,
            dev_today,
        })
    }
}

/// Serves until `shutdown` resolves, then closes the board `WebSocket`s and
/// persists every board that still has unsaved changes.
pub async fn serve(
    config: Config,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    if config.auth.dev_login {
        tracing::warn!("DEV_LOGIN ativo: /auth/dev/{{login}} entra sem GitHub");
    }

    let pool = db::connect(&config.db_url).await?;
    let mut state = AppState::new(pool, config.auth);
    if let Some(day) = &config.dev_today {
        state.clock = crate::clock_on(day, -180, state.clock.now())
            .ok_or_else(|| anyhow::anyhow!("DEV_TODAY inválido: {day}"))?;
        tracing::warn!("DEV_TODAY ativo: o Ritmo acha que hoje é {day}");
    }
    let _cleanup = crate::spawn_session_cleanup(state.clone(), crate::SESSION_CLEANUP_EVERY);
    let (rooms, db) = (state.rooms.clone(), state.db.clone());
    let router = app(state, &config.assets);

    tracing::info!("ouvindo em http://{}", listener.local_addr()?);
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown.await;
            tracing::info!("desligando: salvando quadros e fechando conexões");
            // Closing the WebSockets lets the shutdown finish instead of
            // waiting for them; unsaved board changes are written first.
            rooms.shutdown(&db).await;
        })
        .await?;
    Ok(())
}
