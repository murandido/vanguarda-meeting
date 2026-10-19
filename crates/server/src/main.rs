use std::net::SocketAddr;

use server::{AppState, Assets, app, db};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,loro_internal=warn,server=debug".into()),
        )
        .init();

    let db_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://vanguarda.db".to_owned());
    let addr: SocketAddr = std::env::var("BIND")
        .unwrap_or_else(|_| "127.0.0.1:3000".to_owned())
        .parse()?;

    let mut assets = Assets::default();
    if let Ok(dir) = std::env::var("STATIC_DIR") {
        assets.static_dir = dir.into();
    }
    if let Ok(dir) = std::env::var("PKG_DIR") {
        assets.pkg_dir = dir.into();
    }

    let pool = db::connect(&db_url).await?;
    let router = app(AppState::new(pool), &assets);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("ouvindo em http://{addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
