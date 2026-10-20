use askama::Template;
use axum::{
    Extension,
    extract::Path,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use crate::auth::CurrentUser;

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    user: String,
}

#[derive(Template)]
#[template(path = "board.html")]
struct BoardPage {
    user: String,
    board: String,
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

pub async fn index(Extension(user): Extension<CurrentUser>) -> Response {
    render(&Index { user: user.login })
}

pub async fn board(Extension(user): Extension<CurrentUser>, Path(board): Path<String>) -> Response {
    if !shared::is_valid_board_id(&board) {
        return StatusCode::NOT_FOUND.into_response();
    }
    render(&BoardPage {
        user: user.login,
        board,
    })
}
