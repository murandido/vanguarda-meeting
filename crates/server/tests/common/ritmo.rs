//! Helpers shared by the Ritmo integration tests: two invited users ("ana"
//! and "bia") on an in-memory database, with the server clock fixed.

use axum::{Router, http::StatusCode, response::Response};
use server::Clock;
use sqlx::SqlitePool;

use super::{body_text, dev_auth, dev_session, get, memory_db, post_htmx, router_at};

/// Terça, 20/10/2026, 12:00 em Brasília (15:00 UTC): dia de reunião.
pub const TUE_NOON: i64 = 1_792_508_400;
pub const HOUR: i64 = 3600;
pub const DAY: i64 = 24 * HOUR;
/// Quarta, 21/10/2026, 12:00 em Brasília: primeiro dia da semana 21/10 – 27/10.
pub const WED_NOON: i64 = TUE_NOON + DAY;

pub struct Dupla {
    pub app: Router,
    pub clock: Clock,
    pub pool: SqlitePool,
    pub ana: String,
    pub bia: String,
}

pub async fn dupla(now: i64) -> Dupla {
    let pool = memory_db().await;
    let (app, clock) = router_at(pool.clone(), dev_auth(), now);
    let ana = dev_session(&app, "ana").await;
    let bia = dev_session(&app, "bia").await;
    Dupla {
        app,
        clock,
        pool,
        ana,
        bia,
    }
}

impl Dupla {
    pub async fn post(&self, who: &str, path: &str, form: &str) -> Response {
        post_htmx(&self.app, path, who, form).await
    }

    pub async fn page(&self, who: &str, path: &str) -> String {
        let res = get(&self.app, path, Some(who)).await;
        assert_eq!(res.status(), StatusCode::OK, "GET {path}");
        body_text(res).await
    }

    /// Cria um projeto e devolve o id.
    pub async fn project(&self, who: &str, name: &str) -> i64 {
        let res = self
            .post(
                who,
                "/ritmo/projetos",
                &format!("nome={name}&cor=%23FF8800"),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["hx-refresh"], "true");
        sqlx::query_scalar("SELECT id FROM ritmo_projects WHERE name = ?")
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    /// Cria uma tarefa na semana `semana` e devolve o id.
    pub async fn task(&self, who: &str, semana: &str, project: i64, title: &str) -> i64 {
        let res = self
            .post(
                who,
                "/ritmo/tarefas",
                &format!("semana={semana}&projeto={project}&titulo={title}&tamanho=M"),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "{}", body_text(res).await);
        sqlx::query_scalar("SELECT id FROM ritmo_tasks WHERE title = ?")
            .bind(title.replace('+', " "))
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    pub async fn count(&self, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.unwrap()
    }
}

/// Coluna (`todo`/`doing`/`done`) onde `title` aparece na raia de `login`.
pub fn column_of(html: &str, login: &str, title: &str) -> Option<String> {
    let start = html.find(&format!(r#"<section class="raia" data-login="{login}">"#))?;
    let lane = &html[start..];
    let end = lane[1..]
        .find(r#"<section class="raia""#)
        .or_else(|| lane.find(r#"<section class="feed""#))
        .unwrap_or(lane.len());
    lane[..end]
        .split(r#"<div class="coluna" data-status=""#)
        .skip(1)
        .find(|col| col.contains(&format!(r#"<span class="titulo">{title}</span>"#)))
        .map(|col| col.split('"').next().unwrap().to_owned())
}
