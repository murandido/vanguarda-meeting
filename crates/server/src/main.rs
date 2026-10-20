use std::net::SocketAddr;

use server::{AppState, Assets, AuthConfig, app, db};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Usado pelo healthcheck do Docker: a imagem não tem curl.
    if std::env::args().nth(1).as_deref() == Some("--healthcheck") {
        let port = server::health_port(std::env::var("BIND").ok().as_deref());
        std::process::exit(i32::from(!server::healthcheck(port).await));
    }

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

    let auth = AuthConfig::from_env()?;
    anyhow::ensure!(
        !auth.dev_login || addr.ip().is_loopback(),
        "DEV_LOGIN só é permitido com BIND em loopback (127.0.0.1)"
    );
    if auth.dev_login {
        tracing::warn!("DEV_LOGIN ativo: /auth/dev/{{login}} entra sem GitHub");
    }

    let pool = db::connect(&db_url).await?;
    let router = app(AppState::new(pool, auth), &assets);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("ouvindo em http://{addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
