//! Lista de quadros do Cartazes (CA-09): criar, renomear, arquivar e
//! desarquivar. As ações são POSTs do HTMX que devolvem a lista inteira
//! (`#cartazes`) já atualizada.

use askama::Template;
use axum::{
    Extension, Form,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    AppState,
    auth::{CurrentUser, now},
    db::{self, BoardRow},
    pages::render,
};

/// Tamanho máximo do título, em caracteres.
const MAX_TITLE: usize = 80;
/// Tamanho máximo da parte do slug derivada do título (sobra espaço para `-999`).
const MAX_SLUG_BASE: usize = 48;

struct BoardItem {
    id: String,
    title: String,
    updated_at: String,
    updated_label: String,
}

impl BoardItem {
    fn from_row(row: BoardRow, now: i64) -> Self {
        let updated_label = relative_time(&row.updated_at, now);
        Self {
            id: row.id,
            title: row.title,
            updated_at: row.updated_at,
            updated_label,
        }
    }
}

#[derive(Template)]
#[template(path = "_cartazes.html")]
struct BoardList {
    active: Vec<BoardItem>,
    archived: Vec<BoardItem>,
    error: Option<&'static str>,
}

#[derive(Deserialize)]
pub struct TitleForm {
    titulo: String,
}

/// Verifica se o quadro pode ser aberto: 404 se não existir (ou o nome for
/// inválido), 410 se estiver arquivado.
pub async fn open_board(db: &SqlitePool, id: &str) -> Result<BoardRow, StatusCode> {
    if !shared::is_valid_board_id(id) {
        return Err(StatusCode::NOT_FOUND);
    }
    match db::get_board(db, id).await {
        Ok(Some(board)) if board.is_archived() => Err(StatusCode::GONE),
        Ok(Some(board)) => Ok(board),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::error!("falha ao ler quadro: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Slug do título: minúsculas, sem acentos, `[a-z0-9-]`, no máximo
/// [`MAX_SLUG_BASE`] caracteres. Título sem nada aproveitável vira `quadro`.
#[must_use]
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase).map(fold_accent) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= MAX_SLUG_BASE {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "quadro".to_owned()
    } else {
        slug.to_owned()
    }
}

fn fold_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        _ => c,
    }
}

/// Título limpo, ou a mensagem de erro para mostrar.
fn clean_title(title: &str) -> Result<&str, &'static str> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Dê um título ao quadro.");
    }
    if title.chars().count() > MAX_TITLE {
        return Err("Título longo demais (máximo de 80 caracteres).");
    }
    if title.chars().any(char::is_control) {
        return Err("O título tem caracteres inválidos.");
    }
    Ok(title)
}

/// Segundos Unix de um `AAAA-MM-DDTHH:MM:SS…` em UTC (como o SQLite grava).
fn parse_utc(ts: &str) -> Option<i64> {
    let num = |range: std::ops::Range<usize>| ts.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Dias desde 1970-01-01 (algoritmo `days_from_civil`, de Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3_600 + mm * 60 + ss)
}

/// "agora", "há 5 min", "há 3 h", "há 2 dias" ou a data (`20/10/2026`).
fn relative_time(ts: &str, now: i64) -> String {
    let Some(then) = parse_utc(ts) else {
        return ts.to_owned();
    };
    let secs = (now - then).max(0);
    match secs {
        0..60 => "agora".to_owned(),
        60..3_600 => format!("há {} min", secs / 60),
        3_600..86_400 => format!("há {} h", secs / 3_600),
        86_400..172_800 => "ontem".to_owned(),
        172_800..604_800 => format!("há {} dias", secs / 86_400),
        _ => format!("em {}/{}/{}", &ts[8..10], &ts[5..7], &ts[0..4]),
    }
}

fn is_htmx(headers: &HeaderMap) -> bool {
    headers.get("hx-request").is_some_and(|v| v == "true")
}

async fn render_list(db: &SqlitePool, error: Option<&'static str>) -> Response {
    let rows = match db::list_boards(db).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("falha ao listar quadros: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let now = now();
    let (archived, active): (Vec<_>, Vec<_>) = rows.into_iter().partition(BoardRow::is_archived);
    render(&BoardList {
        active: active
            .into_iter()
            .map(|r| BoardItem::from_row(r, now))
            .collect(),
        archived: archived
            .into_iter()
            .map(|r| BoardItem::from_row(r, now))
            .collect(),
        error,
    })
}

/// `GET /cartazes`: a lista (fragmento HTML carregado pela home).
pub async fn list(State(state): State<AppState>) -> Response {
    render_list(&state.db, None).await
}

/// `POST /cartazes`: cria um quadro com slug único derivado do título.
pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    headers: HeaderMap,
    Form(form): Form<TitleForm>,
) -> Response {
    if !is_htmx(&headers) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let title = match clean_title(&form.titulo) {
        Ok(title) => title,
        Err(msg) => return render_list(&state.db, Some(msg)).await,
    };
    let base = slugify(title);
    for n in 1..=999 {
        let id = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        match db::create_board(&state.db, &id, title, &user.login).await {
            Ok(true) => {
                let mut res = render_list(&state.db, None).await;
                if let Ok(value) = id.parse() {
                    res.headers_mut().insert("x-board-id", value);
                }
                return res;
            }
            Ok(false) => {}
            Err(e) => {
                tracing::error!("falha ao criar quadro: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }
    render_list(
        &state.db,
        Some("Já existem quadros demais com esse título."),
    )
    .await
}

/// `POST /cartazes/{id}/renomear`.
pub async fn rename(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<TitleForm>,
) -> Response {
    if !is_htmx(&headers) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let title = match clean_title(&form.titulo) {
        Ok(title) => title,
        Err(msg) => return render_list(&state.db, Some(msg)).await,
    };
    respond(&state.db, db::rename_board(&state.db, &id, title).await).await
}

/// `POST /cartazes/{id}/arquivar`.
pub async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    set_archived(&state, &id, &headers, true).await
}

/// `POST /cartazes/{id}/desarquivar`.
pub async fn unarchive(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    set_archived(&state, &id, &headers, false).await
}

async fn set_archived(state: &AppState, id: &str, headers: &HeaderMap, archived: bool) -> Response {
    if !is_htmx(headers) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    respond(
        &state.db,
        db::set_board_archived(&state.db, id, archived).await,
    )
    .await
}

/// Resposta de uma alteração: a lista atualizada, ou 404 se o quadro não existe.
async fn respond(db: &SqlitePool, changed: sqlx::Result<bool>) -> Response {
    match changed {
        Ok(true) => render_list(db, None).await,
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("falha ao alterar quadro: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Reunião de Outubro"), "reuniao-de-outubro");
        assert_eq!(slugify("  Ação!! 2026 / Q4  "), "acao-2026-q4");
        assert_eq!(slugify("e2e-123-ab"), "e2e-123-ab");
        assert_eq!(slugify("???"), "quadro");
        assert_eq!(slugify("日本"), "quadro");
        let long = slugify(&"palavra ".repeat(20));
        assert!(long.len() <= MAX_SLUG_BASE && !long.ends_with('-'));
        assert!(shared::is_valid_board_id(&long));
    }

    #[test]
    fn titles() {
        assert_eq!(clean_title("  Planejamento  "), Ok("Planejamento"));
        assert!(clean_title("   ").is_err());
        assert!(clean_title(&"x".repeat(81)).is_err());
        assert!(clean_title(&"ç".repeat(80)).is_ok());
        assert!(clean_title("a\u{0}b").is_err());
    }

    #[test]
    fn parses_sqlite_timestamps() {
        assert_eq!(parse_utc("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_utc("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_utc("2026-10-20T12:34:56.789Z"), Some(1_792_499_696));
        assert_eq!(parse_utc("lixo"), None);
    }

    #[test]
    fn relative_labels() {
        let ts = "2026-10-20T12:00:00.000Z";
        let t = parse_utc(ts).unwrap();
        assert_eq!(relative_time(ts, t - 5), "agora");
        assert_eq!(relative_time(ts, t + 30), "agora");
        assert_eq!(relative_time(ts, t + 5 * 60), "há 5 min");
        assert_eq!(relative_time(ts, t + 3 * 3_600), "há 3 h");
        assert_eq!(relative_time(ts, t + 30 * 3_600), "ontem");
        assert_eq!(relative_time(ts, t + 3 * 86_400), "há 3 dias");
        assert_eq!(relative_time(ts, t + 30 * 86_400), "em 20/10/2026");
        assert_eq!(relative_time("lixo", t), "lixo");
    }
}
