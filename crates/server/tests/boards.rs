//! Lista de quadros do Cartazes (CA-09): criar, renomear, arquivar.

mod common;

use std::borrow::Cow;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{body_text, dev_auth, dev_session, get, hx_post, memory_db, router, send};
use server::db;
use sqlx::sqlite::SqlitePoolOptions;

async fn logged_in() -> (axum::Router, sqlx::SqlitePool, String) {
    let pool = memory_db().await;
    let app = router(pool.clone(), dev_auth());
    let cookie = dev_session(&app, "ana").await;
    (app, pool, cookie)
}

#[tokio::test]
async fn empty_list_has_new_board_form() {
    let (app, _, cookie) = logged_in().await;
    let res = get(&app, "/cartazes", Some(&cookie)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains(r#"id="cartazes""#));
    assert!(body.contains("Novo quadro"));
    assert!(body.contains("Nenhum quadro ainda."));
}

#[tokio::test]
async fn list_needs_login() {
    let app = router(memory_db().await, dev_auth());
    let res = get(&app, "/cartazes", None).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn create_generates_unique_slugs() {
    let (app, pool, cookie) = logged_in().await;
    let res = hx_post(&app, "/cartazes", &cookie, "titulo=Reuni%C3%A3o+de+Outubro").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["x-board-id"], "reuniao-de-outubro");
    let body = body_text(res).await;
    assert!(body.contains(r#"href="/b/reuniao-de-outubro""#));
    assert!(body.contains("Reunião de Outubro"));
    assert!(body.contains("agora"));

    let res = hx_post(&app, "/cartazes", &cookie, "titulo=Reuni%C3%A3o+de+outubro").await;
    assert_eq!(res.headers()["x-board-id"], "reuniao-de-outubro-2");

    let row = db::get_board(&pool, "reuniao-de-outubro")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.created_by.as_deref(), Some("ana"));
    assert!(!row.is_archived());
    assert_eq!(db::list_boards(&pool).await.unwrap().len(), 2);
}

#[tokio::test]
async fn create_rejects_bad_titles_and_plain_posts() {
    let (app, pool, cookie) = logged_in().await;
    let res = hx_post(&app, "/cartazes", &cookie, "titulo=+++").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("Dê um título ao quadro."));

    let long = "x".repeat(81);
    let res = hx_post(&app, "/cartazes", &cookie, &format!("titulo={long}")).await;
    assert!(body_text(res).await.contains("Título longo demais"));

    // Sem o cabeçalho do HTMX (ex.: formulário de outro site): recusado.
    let req = Request::post("/cartazes")
        .header(header::COOKIE, &cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("titulo=Ataque"))
        .unwrap();
    assert_eq!(send(&app, req).await.status(), StatusCode::BAD_REQUEST);
    for path in [
        "/cartazes/x/renomear",
        "/cartazes/x/arquivar",
        "/cartazes/x/desarquivar",
    ] {
        let req = Request::post(path)
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("titulo=x"))
            .unwrap();
        assert_eq!(send(&app, req).await.status(), StatusCode::BAD_REQUEST);
    }

    assert_eq!(db::list_boards(&pool).await.unwrap(), Vec::new());
}

#[tokio::test]
async fn rename_archive_and_unarchive() {
    let (app, pool, cookie) = logged_in().await;
    hx_post(&app, "/cartazes", &cookie, "titulo=Plano").await;

    let res = hx_post(&app, "/cartazes/plano/renomear", &cookie, "titulo=Plano+Q4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("Plano Q4"));
    let res = hx_post(&app, "/cartazes/plano/renomear", &cookie, "titulo=").await;
    assert!(body_text(res).await.contains("Dê um título"));
    let res = get(&app, "/b/plano", Some(&cookie)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("<strong>Plano Q4</strong>"));
    assert!(body.contains(r#"data-board="plano""#));

    let res = hx_post(&app, "/cartazes/plano/arquivar", &cookie, "").await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("Arquivados (1)"));
    assert!(body.contains("/cartazes/plano/desarquivar"));
    assert!(!body.contains(r#"href="/b/plano""#));
    let row = db::get_board(&pool, "plano").await.unwrap().unwrap();
    assert!(row.is_archived());

    let res = get(&app, "/b/plano", Some(&cookie)).await;
    assert_eq!(res.status(), StatusCode::GONE);
    assert!(body_text(res).await.contains("Quadro arquivado"));

    // Arquivar de novo não muda a data de arquivamento.
    hx_post(&app, "/cartazes/plano/arquivar", &cookie, "").await;
    let again = db::get_board(&pool, "plano").await.unwrap().unwrap();
    assert_eq!(again.archived_at, row.archived_at);

    let res = hx_post(&app, "/cartazes/plano/desarquivar", &cookie, "").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(!body_text(res).await.contains("Arquivados"));
    let res = get(&app, "/b/plano", Some(&cookie)).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn unknown_boards_are_404() {
    let (app, _, cookie) = logged_in().await;
    let res = get(&app, "/b/nao-existe", Some(&cookie)).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(body_text(res).await.contains("Quadro não encontrado"));
    for path in [
        "/cartazes/nao-existe/renomear",
        "/cartazes/nao-existe/arquivar",
        "/cartazes/nao-existe/desarquivar",
    ] {
        let res = hx_post(&app, path, &cookie, "titulo=Novo").await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn saving_a_snapshot_bumps_updated_at() {
    let pool = memory_db().await;
    common::create_board(&pool, "q").await;
    sqlx::query("UPDATE boards SET updated_at = '2000-01-01T00:00:00.000Z'")
        .execute(&pool)
        .await
        .unwrap();
    db::save_snapshot(&pool, "q", b"bytes").await.unwrap();
    let row = db::get_board(&pool, "q").await.unwrap().unwrap();
    assert!(row.updated_at.as_str() > "2020");
}

/// A migração cria linhas em `boards` para os quadros que já tinham snapshot.
#[tokio::test]
async fn migration_backfills_existing_snapshots() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let full = sqlx::migrate!("./migrations");
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = Cow::Owned(
        full.migrations
            .iter()
            .filter(|m| m.version < 4)
            .cloned()
            .collect(),
    );
    before.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO board_snapshots (board_id, snapshot, updated_at)
         VALUES ('antigo', x'00', '2026-01-02T03:04:05.000Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    full.run(&pool).await.unwrap();
    let row = db::get_board(&pool, "antigo").await.unwrap().unwrap();
    assert_eq!(row.title, "antigo");
    assert_eq!(row.created_by, None);
    assert_eq!(row.updated_at, "2026-01-02T03:04:05.000Z");
    assert!(!row.is_archived());
}
