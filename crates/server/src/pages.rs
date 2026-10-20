use askama::Template;
use axum::{
    Extension,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use crate::{AppState, auth::CurrentUser, boards};

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    user: String,
    streaks: Option<crate::ritmo::HomeCard>,
}

#[derive(Template)]
#[template(path = "board.html")]
struct BoardPage {
    user: String,
    board: String,
    title: String,
}

#[derive(Template)]
#[template(path = "aviso.html")]
struct Notice {
    user: String,
    title: &'static str,
    message: &'static str,
}

pub fn render(template: &impl Template) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("falha ao renderizar template: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn index(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    let streaks = crate::ritmo::home_card(&state, &user.login).await;
    render(&Index {
        user: user.login,
        streaks,
    })
}

pub async fn board(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(board): Path<String>,
) -> Response {
    let (status, title, message) = match boards::open_board(&state.db, &board).await {
        Ok(row) => {
            return render(&BoardPage {
                user: user.login,
                board: row.id,
                title: row.title,
            });
        }
        Err(StatusCode::NOT_FOUND) => (
            StatusCode::NOT_FOUND,
            "Quadro não encontrado",
            "Esse quadro não existe. Crie um novo na página inicial.",
        ),
        Err(StatusCode::GONE) => (
            StatusCode::GONE,
            "Quadro arquivado",
            "Esse quadro está arquivado. Desarquive-o na página inicial para voltar a usá-lo.",
        ),
        Err(status) => return status.into_response(),
    };
    let mut res = render(&Notice {
        user: user.login,
        title,
        message,
    });
    *res.status_mut() = status;
    res
}
