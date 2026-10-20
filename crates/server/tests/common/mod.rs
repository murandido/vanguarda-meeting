//! Utilitários compartilhados pelos testes de integração do servidor.
#![allow(dead_code)]

use axum::{
    Router,
    body::Body,
    http::{Request, Response, StatusCode, header},
};
use server::{AppState, Assets, AuthConfig, app, db};
use sqlx::SqlitePool;
use tower::ServiceExt;

/// Configuração com `DEV_LOGIN` ligado e "ana" e "bia" convidadas.
pub fn dev_auth() -> AuthConfig {
    AuthConfig::from_vars(|k| match k {
        "ALLOWED_GITHUB_USERS" => Some("ana,bia".into()),
        "DEV_LOGIN" => Some("1".into()),
        _ => None,
    })
    .unwrap()
}

pub async fn memory_db() -> SqlitePool {
    db::connect("sqlite::memory:").await.unwrap()
}

pub fn router(pool: SqlitePool, auth: AuthConfig) -> Router {
    app(AppState::new(pool, auth), &Assets::default())
}

pub async fn send(router: &Router, req: Request<Body>) -> Response<Body> {
    router.clone().oneshot(req).await.unwrap()
}

/// POST de formulário como o HTMX faz (com `HX-Request: true`).
pub async fn hx_post(router: &Router, path: &str, cookie: &str, form: &str) -> Response<Body> {
    let req = Request::post(path)
        .header(header::COOKIE, cookie)
        .header("hx-request", "true")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(form.to_owned()))
        .unwrap();
    send(router, req).await
}

/// Cria um quadro direto no banco (id == título).
pub async fn create_board(pool: &SqlitePool, id: &str) {
    assert!(db::create_board(pool, id, id, "ana").await.unwrap());
}

pub async fn get(router: &Router, path: &str, cookie: Option<&str>) -> Response<Body> {
    let mut req = Request::get(path);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    send(router, req.body(Body::empty()).unwrap()).await
}

pub async fn body_text(res: Response<Body>) -> String {
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn location(res: &Response<Body>) -> &str {
    res.headers()[header::LOCATION].to_str().unwrap()
}

/// Pares `nome=valor` de todos os `Set-Cookie` da resposta.
pub fn set_cookies(res: &Response<Body>) -> Vec<String> {
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
        .collect()
}

/// Entra pelo login de desenvolvimento e devolve o cookie de sessão.
pub async fn dev_session(router: &Router, login: &str) -> String {
    let res = get(router, &format!("/auth/dev/{login}"), None).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    set_cookies(&res)
        .into_iter()
        .find(|c| c.starts_with("vanguarda_session="))
        .expect("cookie de sessão")
}
