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

/// Fake GitHub: the `code` becomes the token `tok-<code>`, and the token
/// becomes the login. The code may carry the account id (`login@id`; 42 by
/// default). The code `falha` makes the token exchange answer 500.
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
        let code = auth.strip_prefix("Bearer tok-").unwrap();
        let (login, id) = code
            .split_once('@')
            .map_or((code, 42), |(l, id)| (l, id.parse().unwrap()));
        assert!(headers.contains_key(header::USER_AGENT));
        Json(json!({ "login": login, "id": id, "name": "Nome", "avatar_url": null }))
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
    oauth_app_with("ana,bia").await
}

async fn oauth_app_with(allowed: &str) -> (Router, sqlx::SqlitePool) {
    let github = fake_github().await;
    let auth = AuthConfig::from_vars(|k| match k {
        "ALLOWED_GITHUB_USERS" => Some(allowed.into()),
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

/// Runs the whole OAuth flow and returns the session cookie, if let in.
async fn oauth_login(app: &Router, code: &str) -> Result<String, String> {
    let (cookie, state) = start(app).await;
    let res = callback(app, &cookie, code, &state).await;
    session_of(&res).ok_or_else(|| location(&res).to_owned())
}

#[tokio::test]
async fn pinned_github_id_must_match() {
    let (app, pool) = oauth_app_with("ana:7,bia").await;
    assert_eq!(
        oauth_login(&app, "ana@42").await.unwrap_err(),
        "/login?erro=nao-convidado"
    );
    assert_eq!(db::github_id_of(&pool, "ana").await.unwrap(), None);
    oauth_login(&app, "ana@7").await.expect("id certo entra");
    // Entries without an id still work (backward compatible).
    oauth_login(&app, "bia@99")
        .await
        .expect("login sem id entra");
}

#[tokio::test]
async fn login_stays_bound_to_first_github_account() {
    let (app, pool) = oauth_app().await;
    oauth_login(&app, "ana@42")
        .await
        .expect("primeiro uso vincula");
    assert_eq!(db::github_id_of(&pool, "ana").await.unwrap(), Some(42));

    // Another account registered "ana" after the original one released it.
    assert_eq!(
        oauth_login(&app, "ana@666").await.unwrap_err(),
        "/login?erro=nao-convidado"
    );
    assert_eq!(db::github_id_of(&pool, "ana").await.unwrap(), Some(42));
    oauth_login(&app, "Ana@42")
        .await
        .expect("a dona continua entrando");
}

#[tokio::test]
async fn renamed_account_moves_its_row() {
    let (app, pool) = oauth_app().await;
    let old = oauth_login(&app, "ana@42").await.unwrap();

    // Account 42 became "bia" on GitHub (and "bia" is invited).
    let new = oauth_login(&app, "bia@42").await.expect("renomeada entra");
    assert_eq!(db::github_id_of(&pool, "bia").await.unwrap(), Some(42));
    assert_eq!(db::github_id_of(&pool, "ana").await.unwrap(), None);
    assert_eq!(location(&get(&app, "/", Some(&old)).await), "/login");
    let res = get(&app, "/", Some(&new)).await;
    assert!(body_text(res).await.contains("<span>bia</span>"));

    // Whoever registers the old name is someone else: a new row.
    oauth_login(&app, "ana@77").await.unwrap();
    assert_eq!(db::github_id_of(&pool, "ana").await.unwrap(), Some(77));
}

#[tokio::test]
async fn rename_onto_someone_elses_login_is_refused() {
    let (app, pool) = oauth_app().await;
    oauth_login(&app, "ana@42").await.unwrap();
    oauth_login(&app, "bia@43").await.unwrap();
    assert_eq!(
        oauth_login(&app, "bia@42").await.unwrap_err(),
        "/login?erro=nao-convidado"
    );
    assert_eq!(db::github_id_of(&pool, "ana").await.unwrap(), Some(42));
    assert_eq!(db::github_id_of(&pool, "bia").await.unwrap(), Some(43));
}

#[tokio::test]
async fn rename_replaces_row_without_github_id() {
    let (app, pool) = oauth_app().await;
    db::upsert_user(&pool, "bia", None, None, None)
        .await
        .unwrap();
    oauth_login(&app, "ana@42").await.unwrap();
    oauth_login(&app, "bia@42")
        .await
        .expect("linha sem id dá lugar");
    assert_eq!(db::github_id_of(&pool, "bia").await.unwrap(), Some(42));

    // Row without an id (development login) and a new account: just bind.
    db::upsert_user(&pool, "ana", None, None, None)
        .await
        .unwrap();
    oauth_login(&app, "ana@5").await.unwrap();
    assert_eq!(db::github_id_of(&pool, "ana").await.unwrap(), Some(5));

    // The development login's upsert never overwrites github_id.
    db::upsert_user(&pool, "ana", Some(6), None, None)
        .await
        .unwrap();
    assert_eq!(db::github_id_of(&pool, "ana").await.unwrap(), Some(5));
}

fn dev_auth_for(allowed: &str) -> AuthConfig {
    AuthConfig::from_vars(|k| match k {
        "ALLOWED_GITHUB_USERS" => Some(allowed.into()),
        "DEV_LOGIN" => Some("1".into()),
        _ => None,
    })
    .unwrap()
}

#[tokio::test]
async fn removing_guest_revokes_access_on_restart() {
    let pool = memory_db().await;
    let before = router(pool.clone(), dev_auth_for("ana,bia"));
    let bia = common::dev_session(&before, "bia").await;
    let ana = common::dev_session(&before, "ana").await;
    let bia_id = bia.split_once('=').unwrap().1.to_owned();

    // "Restart" without bia on the list: her session stops working at once.
    let after = router(pool.clone(), dev_auth_for("ana"));
    assert_eq!(location(&get(&after, "/", Some(&bia)).await), "/login");
    assert_eq!(
        get(&after, "/ws/quadro", Some(&bia)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(get(&after, "/", Some(&ana)).await.status(), StatusCode::OK);

    // The boot cleanup deletes the revoked (and expired) sessions.
    db::create_session(&pool, "velha", "ana", 1).await.unwrap();
    let state = server::AppState::new(pool.clone(), dev_auth_for("ana"));
    assert_eq!(server::auth::prune_sessions(&state).await.unwrap(), 2);
    assert_eq!(db::session_login(&pool, &bia_id, 0).await.unwrap(), None);
    assert_eq!(db::session_login(&pool, "velha", 0).await.unwrap(), None);
    assert_eq!(get(&after, "/", Some(&ana)).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn periodic_cleanup_runs_at_boot_and_repeats() {
    let pool = memory_db().await;
    db::upsert_user(&pool, "ana", None, None, None)
        .await
        .unwrap();
    db::create_session(&pool, "s1", "ana", 1).await.unwrap();
    let state = server::AppState::new(pool.clone(), dev_auth_for("ana"));
    let task = server::spawn_session_cleanup(state, std::time::Duration::from_millis(20));

    let wait = |id: &'static str| {
        let pool = pool.clone();
        async move {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while db::session_login(&pool, id, 0).await.unwrap().is_some() {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("sessão apagada pela limpeza");
        }
    };
    wait("s1").await;
    db::create_session(&pool, "s2", "ana", 1).await.unwrap();
    wait("s2").await;
    task.abort();
}
