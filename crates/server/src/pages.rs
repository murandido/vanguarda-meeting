use askama::Template;
use axum::{
    extract::Path,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

#[derive(Template)]
#[template(path = "index.html")]
struct Index;

#[derive(Template)]
#[template(path = "board.html")]
struct BoardPage {
    board: String,
}

fn render(template: &impl Template) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("falha ao renderizar template: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn index() -> Response {
    render(&Index)
}

pub async fn board(Path(board): Path<String>) -> Response {
    if !shared::is_valid_board_id(&board) {
        return StatusCode::NOT_FOUND.into_response();
    }
    render(&BoardPage { board })
}
