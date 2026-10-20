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
