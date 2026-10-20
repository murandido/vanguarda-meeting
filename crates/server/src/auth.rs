//! Login por convite via GitHub OAuth.
//!
//! Só entra quem estiver em `ALLOWED_GITHUB_USERS`. A sessão é um token
//! aleatório num cookie `HttpOnly`, guardado na tabela `sessions`.
//! Para desenvolvimento e E2E, `DEV_LOGIN=1` libera `/auth/dev/{login}`,
//! que entra sem passar pelo GitHub (só para logins convidados).

use std::{
    fmt::Write as _,
    time::{SystemTime, UNIX_EPOCH},
};

use askama::Template;
use axum::{
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{AppendHeaders, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::{AppState, db, pages::render};

const SESSION_COOKIE: &str = "vanguarda_session";
const STATE_COOKIE: &str = "vanguarda_oauth_state";
const SESSION_SECS: i64 = 30 * 24 * 60 * 60;
const STATE_SECS: i64 = 10 * 60;

/// Rotas que não exigem login.
const PUBLIC_PREFIXES: [&str; 4] = ["/login", "/auth/", "/health", "/static/"];

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Logins do GitHub convidados, em minúsculas.
    pub allowed: Vec<String>,
    /// URL pública do app, sem barra final (base do `redirect_uri`).
    pub public_url: String,
    pub cookie_secure: bool,
    pub dev_login: bool,
    pub github_web: String,
    pub github_api: String,
}

impl AuthConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    /// Lê a configuração de uma fonte de variáveis (testável sem mexer no
    /// ambiente do processo).
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let allowed: Vec<String> = var("ALLOWED_GITHUB_USERS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        anyhow::ensure!(
            !allowed.is_empty(),
            "ALLOWED_GITHUB_USERS vazio: ninguém conseguiria entrar"
        );

        let dev_login = var("DEV_LOGIN").is_some_and(|v| v == "1" || v == "true");
        let client_id = var("GITHUB_CLIENT_ID").unwrap_or_default();
        let client_secret = var("GITHUB_CLIENT_SECRET").unwrap_or_default();
        anyhow::ensure!(
            dev_login || (!client_id.is_empty() && !client_secret.is_empty()),
            "defina GITHUB_CLIENT_ID e GITHUB_CLIENT_SECRET (ou DEV_LOGIN=1 em desenvolvimento)"
        );

        let public_url = var("PUBLIC_URL")
            .unwrap_or_else(|| "http://127.0.0.1:3000".to_owned())
            .trim_end_matches('/')
            .to_owned();
        let cookie_secure = var("COOKIE_SECURE").map_or_else(
            || public_url.starts_with("https://"),
            |v| v == "1" || v == "true",
        );

        Ok(Self {
            client_id,
            client_secret,
            allowed,
            public_url,
            cookie_secure,
            dev_login,
            github_web: var("GITHUB_WEB_URL").unwrap_or_else(|| "https://github.com".to_owned()),
            github_api: var("GITHUB_API_URL")
                .unwrap_or_else(|| "https://api.github.com".to_owned()),
        })
    }

    fn is_allowed(&self, login: &str) -> bool {
        self.allowed.iter().any(|a| a.eq_ignore_ascii_case(login))
    }

    fn cookie(&self, name: &str, value: &str, max_age: i64) -> String {
        let secure = if self.cookie_secure { "; Secure" } else { "" };
        format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
    }
}

/// Usuário logado, colocado nas extensões da requisição pelo [`require_user`].
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub login: String,
}

pub(crate) fn now() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio depois de 1970")
        .as_secs();
    i64::try_from(secs).expect("cabe em i64")
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("fonte de aleatoriedade do sistema");
    bytes.iter().fold(String::with_capacity(64), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    })
}

fn read_cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then_some(v))
}

/// Middleware: exige sessão válida fora das rotas públicas.
pub async fn require_user(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if PUBLIC_PREFIXES.iter().any(|p| path.starts_with(p)) {
        return next.run(req).await;
    }

    let login = match read_cookie(req.headers(), SESSION_COOKIE) {
        Some(id) => db::session_login(&state.db, id, now()).await,
        None => Ok(None),
    };
    match login {
        Ok(Some(login)) => {
            req.extensions_mut().insert(CurrentUser { login });
            next.run(req).await
        }
        Ok(None) if path.starts_with("/ws/") => StatusCode::UNAUTHORIZED.into_response(),
        Ok(None) => Redirect::to("/login").into_response(),
        Err(e) => {
            tracing::error!("falha ao ler sessão: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginPage<'a> {
    error: Option<&'a str>,
    dev_users: &'a [String],
}

#[derive(Deserialize)]
pub struct LoginQuery {
    erro: Option<String>,
}

pub async fn login_page(State(state): State<AppState>, Query(q): Query<LoginQuery>) -> Response {
    let error = match q.erro.as_deref() {
        Some("nao-convidado") => Some("Esse usuário do GitHub não está na lista de convidados."),
        Some(_) => Some("Não foi possível entrar. Tente de novo."),
        None => None,
    };
    let dev_users: &[String] = if state.auth.dev_login {
        &state.auth.allowed
    } else {
        &[]
    };
    render(&LoginPage { error, dev_users })
}

pub async fn github_start(State(state): State<AppState>) -> Response {
    let auth = &state.auth;
    let oauth_state = random_token();
    let redirect_uri = format!("{}/auth/github/callback", auth.public_url);
    let url = match reqwest::Url::parse_with_params(
        &format!("{}/login/oauth/authorize", auth.github_web),
        [
            ("client_id", auth.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("state", oauth_state.as_str()),
            ("allow_signup", "false"),
        ],
    ) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!("GITHUB_WEB_URL inválida: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    (
        AppendHeaders([(
            header::SET_COOKIE,
            auth.cookie(STATE_COOKIE, &oauth_state, STATE_SECS),
        )]),
        Redirect::to(url.as_str()),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: String,
    state: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct GithubUser {
    login: String,
    id: i64,
    name: Option<String>,
    avatar_url: Option<String>,
}

async fn fetch_github_user(state: &AppState, code: &str) -> anyhow::Result<GithubUser> {
    let auth = &state.auth;
    let token: TokenResponse = state
        .http
        .post(format!("{}/login/oauth/access_token", auth.github_web))
        .header(header::ACCEPT, "application/json")
        .form(&[
            ("client_id", auth.client_id.as_str()),
            ("client_secret", auth.client_secret.as_str()),
            ("code", code),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let user = state
        .http
        .get(format!("{}/user", auth.github_api))
        .bearer_auth(&token.access_token)
        .header(header::USER_AGENT, "vanguarda")
        .header(header::ACCEPT, "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(user)
}

pub async fn github_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Response {
    if read_cookie(&headers, STATE_COOKIE) != Some(q.state.as_str()) {
        return (StatusCode::BAD_REQUEST, "state do OAuth inválido").into_response();
    }
    let clear_state = state.auth.cookie(STATE_COOKIE, "", 0);

    let user = match fetch_github_user(&state, &q.code).await {
        Ok(user) => user,
        Err(e) => {
            tracing::warn!("falha no OAuth do GitHub: {e:#}");
            return (
                AppendHeaders([(header::SET_COOKIE, clear_state)]),
                Redirect::to("/login?erro=github"),
            )
                .into_response();
        }
    };
    if !state.auth.is_allowed(&user.login) {
        tracing::warn!(login = user.login, "login recusado: não convidado");
        return (
            AppendHeaders([(header::SET_COOKIE, clear_state)]),
            Redirect::to("/login?erro=nao-convidado"),
        )
            .into_response();
    }

    let login = user.login.to_lowercase();
    let result = db::upsert_user(
        &state.db,
        &login,
        Some(user.id),
        user.name.as_deref(),
        user.avatar_url.as_deref(),
    )
    .await;
    match result {
        Ok(()) => start_session(&state, &login, Some(clear_state)).await,
        Err(e) => {
            tracing::error!("falha ao salvar usuário: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Login de desenvolvimento: só existe com `DEV_LOGIN=1`.
pub async fn dev_login(State(state): State<AppState>, Path(login): Path<String>) -> Response {
    if !state.auth.dev_login {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !state.auth.is_allowed(&login) {
        return Redirect::to("/login?erro=nao-convidado").into_response();
    }
    let login = login.to_lowercase();
    if let Err(e) = db::upsert_user(&state.db, &login, None, None, None).await {
        tracing::error!("falha ao salvar usuário: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    start_session(&state, &login, None).await
}

async fn start_session(state: &AppState, login: &str, extra_cookie: Option<String>) -> Response {
    let id = random_token();
    if let Err(e) = db::create_session(&state.db, &id, login, now() + SESSION_SECS).await {
        tracing::error!("falha ao criar sessão: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let session = state.auth.cookie(SESSION_COOKIE, &id, SESSION_SECS);
    let mut res = Redirect::to("/").into_response();
    for cookie in std::iter::once(session).chain(extra_cookie) {
        res.headers_mut().append(
            header::SET_COOKIE,
            cookie.parse().expect("cookie é ASCII válido"),
        );
    }
    res
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(id) = read_cookie(&headers, SESSION_COOKIE)
        && let Err(e) = db::delete_session(&state.db, id).await
    {
        tracing::error!("falha ao apagar sessão: {e}");
    }
    (
        AppendHeaders([(header::SET_COOKIE, state.auth.cookie(SESSION_COOKIE, "", 0))]),
        Redirect::to("/login"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn config_requires_guests_and_credentials() {
        assert!(AuthConfig::from_vars(vars(&[])).is_err());
        assert!(AuthConfig::from_vars(vars(&[("ALLOWED_GITHUB_USERS", "ana")])).is_err());
        assert!(
            AuthConfig::from_vars(vars(&[("ALLOWED_GITHUB_USERS", "ana"), ("DEV_LOGIN", "1")]))
                .is_ok()
        );
    }

    #[test]
    fn config_normalizes_and_infers_secure_cookie() {
        let cfg = AuthConfig::from_vars(vars(&[
            ("ALLOWED_GITHUB_USERS", " Ana , ,bia"),
            ("GITHUB_CLIENT_ID", "id"),
            ("GITHUB_CLIENT_SECRET", "segredo"),
            ("PUBLIC_URL", "https://vanguarda.exemplo/"),
        ]))
        .unwrap();
        assert_eq!(cfg.allowed, ["ana", "bia"]);
        assert_eq!(cfg.public_url, "https://vanguarda.exemplo");
        assert!(cfg.cookie_secure);
        assert!(!cfg.dev_login);
        assert!(cfg.is_allowed("ANA"));
        assert!(!cfg.is_allowed("carla"));
        assert!(cfg.cookie("c", "v", 10).ends_with("; Secure"));
    }

    #[test]
    fn reads_cookie_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "a=1; vanguarda_session=abc; b=2".parse().unwrap(),
        );
        assert_eq!(read_cookie(&headers, SESSION_COOKIE), Some("abc"));
        assert_eq!(read_cookie(&headers, STATE_COOKIE), None);
    }

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
