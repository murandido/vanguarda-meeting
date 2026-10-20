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
}

#[tokio::test]
async fn board_page_validates_name_and_carries_user() {
    let (app, cookie) = logged_in().await;
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
