mod common;

use axum::http::StatusCode;
use common::{body_text, dev_auth, dev_session, get, memory_db, router};

async fn logged_in() -> (axum::Router, String) {
    let app = router(memory_db().await, dev_auth());
    let cookie = dev_session(&app, "ana").await;
    (app, cookie)
}

#[tokio::test]
async fn health_is_public() {
    let app = router(memory_db().await, dev_auth());
    let res = get(&app, "/health", None).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_text(res).await, "ok");
}

#[tokio::test]
async fn index_renders_both_modules_and_user() {
    let (app, cookie) = logged_in().await;
    let res = get(&app, "/", Some(&cookie)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("Cartazes"));
    assert!(body.contains("Ritmo"));
    assert!(body.contains("/static/htmx.min.js"));
    assert!(body.contains("<span>ana</span>"));
    // A lista de quadros vem por HTMX.
    assert!(body.contains(r#"hx-get="/cartazes""#));
}

#[tokio::test]
async fn board_page_validates_name_and_carries_user() {
    let pool = memory_db().await;
    common::create_board(&pool, "reuniao-1").await;
    let app = router(pool, dev_auth());
    let cookie = dev_session(&app, "ana").await;
    let res = get(&app, "/b/reuniao-1", Some(&cookie)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains(r#"data-board="reuniao-1" data-user="ana""#));

    let res = get(&app, "/b/Nome%20Ruim", Some(&cookie)).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn static_assets_are_public() {
    let app = router(memory_db().await, dev_auth());
    let res = get(&app, "/static/board.js", None).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("takePendingUpdate"));
}

#[tokio::test]
async fn healthcheck_probe() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = router(memory_db().await, dev_auth());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    assert!(server::healthcheck(port).await);

    // Porta sem ninguém ouvindo.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = closed.local_addr().unwrap().port();
    drop(closed);
    assert!(!server::healthcheck(dead).await);
}

#[test]
fn health_port_from_bind() {
    assert_eq!(server::health_port(Some("0.0.0.0:8080")), 8080);
    assert_eq!(server::health_port(Some("lixo")), 3000);
    assert_eq!(server::health_port(None), 3000);
}

#[tokio::test]
async fn healthcheck_rejects_non_ok_answer() {
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = socket
            .write_all(b"HTTP/1.1 503 Service Unavailable\r\n\r\nbusy")
            .await;
    });
    assert!(!server::healthcheck(port).await);
}

/// O binário com `--healthcheck`, como o Docker o chama em produção.
/// `multi_thread`: o processo filho bloqueia uma thread enquanto o servidor
/// do teste responde em outra.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn binary_healthcheck_exit_code() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(memory_db().await, dev_auth());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let run = |bind: String| {
        std::process::Command::new(env!("CARGO_BIN_EXE_server"))
            .arg("--healthcheck")
            .env("BIND", bind)
            .status()
            .unwrap()
    };
    assert!(run(addr.to_string()).success());

    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = closed.local_addr().unwrap();
    drop(closed);
    assert_eq!(run(dead.to_string()).code(), Some(1));
}

/// A server that accepts the connection and never answers does not hang the client.
#[tokio::test]
async fn http_client_times_out() {
    use std::time::{Duration, Instant};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let client = server::http_client(Duration::from_millis(200), Duration::from_millis(300));
    let started = Instant::now();
    let err = client
        .get(format!("http://{addr}/"))
        .send()
        .await
        .unwrap_err();
    assert!(err.is_timeout(), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn security_headers_on_every_response() {
    let (app, cookie) = logged_in().await;
    for (path, cookie) in [
        ("/", Some(cookie.as_str())),
        ("/login", None),
        ("/health", None),
        ("/static/app.css", None),
        ("/b/Nome%20Ruim", Some(cookie.as_str())),
        ("/", None),
    ] {
        let res = get(&app, path, cookie).await;
        for (name, value) in server::SECURITY_HEADERS {
            assert_eq!(res.headers()[name], value, "{name} on {path}");
        }
    }
    let csp = server::CONTENT_SECURITY_POLICY;
    assert!(!csp.contains("unsafe-inline") && !csp.contains("'unsafe-eval'"));
    assert!(csp.contains("frame-ancestors 'none'"));
}

/// Every page renders without inline code (the Ritmo pages with data are
/// covered in `tests/ritmo.rs`).
#[tokio::test]
async fn pages_have_no_inline_scripts_or_styles() {
    let pool = memory_db().await;
    common::create_board(&pool, "quadro").await;
    let app = router(pool, dev_auth());
    let cookie = dev_session(&app, "ana").await;
    for path in [
        "/",
        "/cartazes",
        "/b/quadro",
        "/b/nao-existe",
        "/ritmo",
        "/ritmo/checkin",
        "/login",
    ] {
        let html = body_text(get(&app, path, Some(&cookie)).await).await;
        common::assert_csp_safe(&html, path);
    }
    let home = body_text(get(&app, "/", Some(&cookie)).await).await;
    assert!(home.contains(
        r#"<meta name="htmx-config" content='{"includeIndicatorStyles":false,"allowEval":false}'>"#
    ));
}
