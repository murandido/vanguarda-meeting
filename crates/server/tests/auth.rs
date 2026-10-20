//! Fluxo de login por convite contra um GitHub falso (servidor local).

mod common;

use axum::{
    Form, Json, Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
    routing::{get as route_get, post as route_post},
};
use common::{body_text, get, location, memory_db, router, send, set_cookies};
use serde_json::{Value, json};
use server::{AuthConfig, db};
use std::collections::HashMap;
use tokio::net::TcpListener;

/// GitHub falso: o `code` vira o token `tok-<code>`, e o token vira o login.
/// O code `falha` faz a troca de token devolver 500.
async fn fake_github() -> String {
    async fn token(Form(form): Form<HashMap<String, String>>) -> Result<Json<Value>, StatusCode> {
        assert_eq!(form["client_id"], "cid");
        assert_eq!(form["client_secret"], "segredo");
        match form["code"].as_str() {
            "falha" => Err(StatusCode::INTERNAL_SERVER_ERROR),
            code => Ok(Json(json!({ "access_token": format!("tok-{code}") }))),
        }
    }
    async fn user(headers: HeaderMap) -> Json<Value> {
        let auth = headers[header::AUTHORIZATION].to_str().unwrap();
        let login = auth.strip_prefix("Bearer tok-").unwrap();
        assert!(headers.contains_key(header::USER_AGENT));
        Json(json!({ "login": login, "id": 42, "name": "Nome", "avatar_url": null }))
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/login/oauth/access_token", route_post(token))
        .route("/user", route_get(user));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

async fn oauth_app() -> (Router, sqlx::SqlitePool) {
    let github = fake_github().await;
    let auth = AuthConfig::from_vars(|k| match k {
        "ALLOWED_GITHUB_USERS" => Some("ana,bia".into()),
        "GITHUB_CLIENT_ID" => Some("cid".into()),
        "GITHUB_CLIENT_SECRET" => Some("segredo".into()),
        "PUBLIC_URL" => Some("https://vanguarda.exemplo".into()),
        "GITHUB_WEB_URL" | "GITHUB_API_URL" => Some(github.clone()),
        _ => None,
    })
    .unwrap();
    let pool = memory_db().await;
    (router(pool.clone(), auth), pool)
}

/// Inicia o OAuth e devolve o cookie de `state` e o valor do `state`.
async fn start(app: &Router) -> (String, String) {
    let res = get(app, "/auth/github", None).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let url = reqwest::Url::parse(location(&res)).unwrap();
    assert!(url.path().ends_with("/login/oauth/authorize"));
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(params["client_id"], "cid");
    assert_eq!(
        params["redirect_uri"],
        "https://vanguarda.exemplo/auth/github/callback"
    );
    let cookie = set_cookies(&res).remove(0);
    assert!(cookie.starts_with("vanguarda_oauth_state="));
    // PUBLIC_URL em https: cookies com Secure.
    assert!(
        res.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Secure")
    );
    (cookie, params["state"].clone())
}

async fn callback(app: &Router, cookie: &str, code: &str, state: &str) -> axum::response::Response {
    let req = Request::get(format!("/auth/github/callback?code={code}&state={state}"))
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    send(app, req).await
}

fn session_of(res: &axum::response::Response) -> Option<String> {
    set_cookies(res)
        .into_iter()
        .find(|c| c.starts_with("vanguarda_session=") && c.len() > "vanguarda_session=".len())
}

#[tokio::test]
async fn protected_routes_need_login() {
    let (app, _) = oauth_app().await;
    let res = get(&app, "/", None).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/login");

    let res = get(&app, "/ws/qualquer", None).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let res = get(&app, "/", Some("vanguarda_session=inventada")).await;
    assert_eq!(location(&res), "/login");
}

#[tokio::test]
async fn login_page_has_github_button_and_no_dev_links() {
    let (app, _) = oauth_app().await;
    let res = get(&app, "/login?erro=nao-convidado", None).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains(r#"href="/auth/github""#));
    assert!(body.contains("lista de convidados"));
    assert!(!body.contains("/auth/dev/"));
}

#[tokio::test]
async fn invited_user_logs_in_and_out() {
    let (app, _) = oauth_app().await;
    let (cookie, state) = start(&app).await;

    // Login no GitHub com maiúsculas: comparação ignora caixa.
    let res = callback(&app, &cookie, "Ana", &state).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/");
    let session = session_of(&res).expect("sessão criada");

    let res = get(&app, "/", Some(&session)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("<span>ana</span>"));

    let req = Request::post("/logout")
        .header(header::COOKIE, &session)
        .body(Body::empty())
        .unwrap();
    let res = send(&app, req).await;
    assert_eq!(location(&res), "/login");
    assert!(set_cookies(&res).contains(&"vanguarda_session=".to_owned()));

    let res = get(&app, "/", Some(&session)).await;
    assert_eq!(location(&res), "/login", "sessão apagada no logout");
}

#[tokio::test]
async fn uninvited_user_is_refused() {
    let (app, _) = oauth_app().await;
    let (cookie, state) = start(&app).await;
    let res = callback(&app, &cookie, "intruso", &state).await;
    assert_eq!(location(&res), "/login?erro=nao-convidado");
    assert!(session_of(&res).is_none());
}

#[tokio::test]
async fn wrong_state_is_rejected() {
    let (app, _) = oauth_app().await;
    let (cookie, _) = start(&app).await;
    let res = callback(&app, &cookie, "ana", "outro-state").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let req = Request::get("/auth/github/callback?code=ana&state=x")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, req).await.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn github_failure_goes_back_to_login() {
    let (app, _) = oauth_app().await;
    let (cookie, state) = start(&app).await;
    let res = callback(&app, &cookie, "falha", &state).await;
    assert_eq!(location(&res), "/login?erro=github");
    assert!(session_of(&res).is_none());

    let res = get(&app, "/login?erro=github", None).await;
    assert!(body_text(res).await.contains("Tente de novo"));
}

#[tokio::test]
async fn dev_login_is_off_by_default() {
    let (app, _) = oauth_app().await;
    let res = get(&app, "/auth/dev/ana", None).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dev_login_only_for_guests() {
    let app = router(memory_db().await, common::dev_auth());
    let res = get(&app, "/auth/dev/intruso", None).await;
    assert_eq!(location(&res), "/login?erro=nao-convidado");

    let res = get(&app, "/login", None).await;
    let body = body_text(res).await;
    assert!(body.contains(r#"href="/auth/dev/ana""#));
    assert!(body.contains(r#"href="/auth/dev/bia""#));
}

#[tokio::test]
async fn expired_session_is_rejected() {
    let (app, pool) = oauth_app().await;
    db::upsert_user(&pool, "ana", None, None, None)
        .await
        .unwrap();
    db::create_session(&pool, "velha", "ana", 1).await.unwrap();
    let res = get(&app, "/", Some("vanguarda_session=velha")).await;
    assert_eq!(location(&res), "/login");
}
