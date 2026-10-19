use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use server::{AppState, Assets, app, db};
use tower::ServiceExt;

async fn test_app() -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    app(AppState::new(pool), &Assets::default())
}

async fn get(path: &str) -> (StatusCode, String) {
    let res = test_app()
        .await
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn health_is_ok() {
    assert_eq!(get("/health").await, (StatusCode::OK, "ok".into()));
}

#[tokio::test]
async fn index_renders_both_modules() {
    let (status, body) = get("/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Cartazes"));
    assert!(body.contains("Ritmo"));
    assert!(body.contains("/static/htmx.min.js"));
}

#[tokio::test]
async fn board_page_escapes_and_validates_name() {
    let (status, body) = get("/b/reuniao-1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"data-board="reuniao-1""#));

    let (status, _) = get("/b/Nome%20Ruim").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn static_assets_are_served() {
    let (status, body) = get("/static/board.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("takePendingUpdate"));
}
