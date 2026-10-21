use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Used by the Docker healthcheck: the image has no curl.
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

    let config = server::Config::from_env()?;
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    server::serve(config, listener, shutdown_signal()).await
}

/// Ctrl+C or SIGTERM (what `docker stop` sends).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}
