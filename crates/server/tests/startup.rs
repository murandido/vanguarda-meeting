//! Configuration from the environment and the serve loop's graceful shutdown.

use std::time::Duration;

use server::{Config, serve};
use tokio::{net::TcpListener, sync::oneshot};

fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| {
        pairs
            .iter()
            .find(|(name, _)| *name == k)
            .map(|(_, v)| (*v).to_owned())
    }
}

const DEV: [(&str, &str); 2] = [("ALLOWED_GITHUB_USERS", "ana"), ("DEV_LOGIN", "1")];

#[test]
fn config_defaults() {
    let config = Config::from_vars(vars(&DEV)).unwrap();
    assert_eq!(config.db_url, "sqlite://vanguarda.db");
    assert_eq!(config.addr.to_string(), "127.0.0.1:3000");
    assert!(config.assets.static_dir.ends_with("static"));
    assert!(config.auth.dev_login);
}

#[test]
fn config_overrides() {
    let config = Config::from_vars(vars(&[
        ("ALLOWED_GITHUB_USERS", "ana"),
        ("GITHUB_CLIENT_ID", "id"),
        ("GITHUB_CLIENT_SECRET", "secret"),
        ("DATABASE_URL", "sqlite::memory:"),
        ("BIND", "0.0.0.0:8080"),
        ("STATIC_DIR", "/srv/static"),
        ("PKG_DIR", "/srv/pkg"),
    ]))
    .unwrap();
    assert_eq!(config.db_url, "sqlite::memory:");
    assert_eq!(config.addr.port(), 8080);
    assert_eq!(config.assets.static_dir.to_str(), Some("/srv/static"));
    assert_eq!(config.assets.pkg_dir.to_str(), Some("/srv/pkg"));
}

#[test]
fn config_rejects_bad_input() {
    // Invalid bind address.
    let bad_bind = [
        ("ALLOWED_GITHUB_USERS", "ana"),
        ("DEV_LOGIN", "1"),
        ("BIND", "nope"),
    ];
    assert!(Config::from_vars(vars(&bad_bind)).is_err());

    // DEV_LOGIN is only allowed on loopback.
    let exposed = [
        ("ALLOWED_GITHUB_USERS", "ana"),
        ("DEV_LOGIN", "1"),
        ("BIND", "0.0.0.0:3000"),
    ];
    let err = Config::from_vars(vars(&exposed)).err().unwrap();
    assert!(err.to_string().contains("DEV_LOGIN"));

    // Auth errors propagate (no guests).
    assert!(Config::from_vars(vars(&[("DEV_LOGIN", "1")])).is_err());
}

#[tokio::test]
async fn serves_until_shutdown() {
    let mut config = Config::from_vars(vars(&DEV)).unwrap();
    config.db_url = "sqlite::memory:".to_owned();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(serve(config, listener, async {
        let _ = stopped.await;
    }));

    // Wait until the server answers its health check.
    let mut healthy = false;
    for _ in 0..50 {
        if server::healthcheck(port).await {
            healthy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(healthy, "server never became healthy");

    stop.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("shutdown finishes promptly")
        .unwrap();
    assert!(result.is_ok());
    assert!(
        !server::healthcheck(port).await,
        "port is closed after shutdown"
    );
}

#[tokio::test]
async fn serve_fails_on_unreachable_database() {
    let mut config = Config::from_vars(vars(&DEV)).unwrap();
    config.db_url = "sqlite:///nonexistent-dir/for/sure/vanguarda.db".to_owned();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(
        serve(config, listener, std::future::pending())
            .await
            .is_err()
    );
}
