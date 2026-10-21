//! Ritmo week closing (KB-07): summary, carry-over or drop, idempotency,
//! authorization and read-only weeks afterwards.

mod common;

use axum::http::StatusCode;
use common::{
    body_text,
    ritmo::{DAY, Dupla, WED_NOON, column_of, dupla},
};

/// Tuesday 27/10/2026, noon in Brasília: the meeting that ends the week
/// 21/10 – 27/10.
const MEETING: i64 = WED_NOON + 6 * DAY;
const WEEK: &str = "2026-10-21";
const CLOSE: &str = "/ritmo/semana/fechar";

/// Ana's week: three promised tasks (one done, one blocked in a check-in
/// and still in progress, one untouched) plus one extra done after
/// confirming. Bia has one task of her own.
struct Semana {
    d: Dupla,
    done: i64,
    blocked: i64,
    untouched: i64,
    extra: i64,
    bias: i64,
}

async fn semana() -> Semana {
    let d = dupla(WED_NOON).await;
    let p = d.project(&d.ana, "Livro").await;
    let q = d.project(&d.bia, "Corrida").await;
    let done = d.task(&d.ana, WEEK, p, "Feita").await;
    let blocked = d.task(&d.ana, WEEK, p, "Travada").await;
    let untouched = d.task(&d.ana, WEEK, p, "Parada").await;
    let res = d
        .post(&d.ana, "/ritmo/semana/confirmar", &format!("semana={WEEK}"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let extra = d.task(&d.ana, WEEK, p, "Extra").await;
    let bias = d.task(&d.bia, WEEK, q, "Correr").await;

    d.post(&d.ana, &format!("/ritmo/tarefas/{done}/mover"), "para=done")
        .await;
    d.post(
        &d.ana,
        &format!("/ritmo/tarefas/{extra}/mover"),
        "para=done",
    )
    .await;
    d.post(
        &d.ana,
        &format!("/ritmo/tarefas/{blocked}/mover"),
        "para=doing",
    )
    .await;
    let res = d
        .post(
            &d.ana,
            "/ritmo/checkin",
            &format!("status-{blocked}=blocked&status-{untouched}=untouched"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    Semana {
        d,
        done,
        blocked,
        untouched,
        extra,
        bias,
    }
}

#[tokio::test]
async fn closing_waits_for_the_meeting_day() {
    let s = semana().await;
    let d = &s.d;
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(!body.contains("Fechar semana"));

    // The page is a preview with the automatic summary.
    let body = d.page(&d.ana, &format!("{CLOSE}?semana={WEEK}")).await;
    assert!(body.contains("a partir de 27/10"));
    assert!(!body.contains(r#"id="form-fechar""#));

    let res = d
        .post(&d.ana, CLOSE, &format!("semana={WEEK}&todas=levar"))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_week_closings").await, 0);
}

#[tokio::test]
async fn summary_shows_promised_done_extras_and_blocked() {
    let s = semana().await;
    let d = &s.d;
    d.clock.set(MEETING);
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains(&format!(r#"href="{CLOSE}?semana={WEEK}">Fechar semana"#)));

    let body = d.page(&d.ana, &format!("{CLOSE}?semana={WEEK}")).await;
    let ana =
        &body[body.find(r#"data-login="ana""#).unwrap()..body.find(r#"data-login="bia""#).unwrap()];
    assert!(ana.contains("<dt>Prometido</dt><dd>3</dd>"));
    assert!(ana.contains("<dt>Feito do prometido</dt><dd>1 <small>(33%)</small></dd>"));
    assert!(ana.contains("<dt>Extras feitos</dt><dd>1</dd>"));
    assert!(ana.contains("<dt>Travadas</dt><dd>1</dd>"));
    assert!(ana.contains("Travaram na semana: Travada"));
    // One choice per unfinished task, and only for them.
    assert!(ana.contains(&format!(r#"name="destino-{}""#, s.blocked)));
    assert!(ana.contains(&format!(r#"name="destino-{}""#, s.untouched)));
    assert!(!ana.contains(&format!(r#"name="destino-{}""#, s.done)));
    assert!(!ana.contains(&format!(r#"name="destino-{}""#, s.extra)));

    // Bia's summary is visible, but without a form for Ana.
    let bia = &body[body.find(r#"data-login="bia""#).unwrap()..];
    assert!(bia.contains("ainda não fechou"));
    assert!(!bia.contains("destino-"));
    assert!(bia.contains("<dt>Prometido</dt><dd>0</dd>"));
}

#[tokio::test]
async fn closing_carries_and_drops_and_is_stored() {
    let s = semana().await;
    let d = &s.d;
    d.clock.set(MEETING);

    // Every unfinished task needs a decision.
    let res = d
        .post(
            &d.ana,
            CLOSE,
            &format!("semana={WEEK}&destino-{}=levar", s.blocked),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(body_text(res).await.contains("cada tarefa"));

    let res = d
        .post(
            &d.ana,
            CLOSE,
            &format!(
                "semana={WEEK}&destino-{}=levar&destino-{}=descartar",
                s.blocked, s.untouched
            ),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["hx-redirect"], "/ritmo?semana=2026-10-28");

    let stored: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT promised, promised_done, extras_done, blocked, carried, dropped
         FROM ritmo_week_closings WHERE login = 'ana'",
    )
    .fetch_one(&d.pool)
    .await
    .unwrap();
    assert_eq!(stored, (3, 1, 1, 1, 1, 1));
    let outcomes: Vec<(String, String, bool, bool)> = sqlx::query_as(
        "SELECT title, outcome, promised, was_blocked FROM ritmo_week_closing_tasks ORDER BY id",
    )
    .fetch_all(&d.pool)
    .await
    .unwrap();
    let row = |t: &str, o: &str, p: bool, b: bool| (t.to_owned(), o.to_owned(), p, b);
    assert_eq!(
        outcomes,
        [
            row("Feita", "done", true, false),
            row("Travada", "carried", true, true),
            row("Parada", "dropped", true, false),
            row("Extra", "done", false, false),
        ]
    );

    // The carried task shows up next week, in the same column, marked.
    let (carried_id, status, from): (i64, String, i64) = sqlx::query_as(
        "SELECT t.id, t.status, t.carried_from FROM ritmo_tasks t
         JOIN ritmo_weeks w ON w.id = t.week_id
         WHERE w.start_date = '2026-10-28'",
    )
    .fetch_one(&d.pool)
    .await
    .unwrap();
    assert_eq!((status.as_str(), from), ("doing", s.blocked));
    let body = d.page(&d.ana, "/ritmo?semana=2026-10-28").await;
    assert_eq!(column_of(&body, "ana", "Travada").as_deref(), Some("doing"));
    assert!(body.contains("↪ levada"));
    // It is a normal task of the new week.
    let res = d
        .post(
            &d.ana,
            &format!("/ritmo/tarefas/{carried_id}/mover"),
            "para=done",
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    // The closed page shows what happened to each task.
    let body = d.page(&d.bia, &format!("{CLOSE}?semana={WEEK}")).await;
    assert!(body.contains("fechou 27/10"));
    assert!(body.contains(r#"data-outcome="carried""#));
    assert!(body.contains("descartada · prometida"));
    assert!(body.contains("<dt>Levadas</dt><dd>1</dd>"));
}

#[tokio::test]
async fn closing_is_idempotent() {
    let s = semana().await;
    let d = &s.d;
    d.clock.set(MEETING);
    let form = format!("semana={WEEK}&todas=levar");
    let res = d.post(&d.ana, CLOSE, &form).await;
    assert_eq!(res.status(), StatusCode::OK);
    let tasks = d.count("SELECT COUNT(*) FROM ritmo_tasks").await;
    assert_eq!(tasks, 5 + 2);

    // Again, even with other choices: nothing changes.
    let res = d.post(&d.ana, CLOSE, &form).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()["hx-redirect"],
        format!("/ritmo?semana={WEEK}")
    );
    let res = d
        .post(&d.ana, CLOSE, &format!("semana={WEEK}&todas=descartar"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_tasks").await, tasks);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_week_closings").await, 1);
    assert_eq!(
        d.count("SELECT COUNT(*) FROM ritmo_week_closing_tasks")
            .await,
        4
    );
}

#[tokio::test]
async fn cannot_close_or_carry_the_other_persons_tasks() {
    let s = semana().await;
    let d = &s.d;
    d.clock.set(MEETING);
    // Bia tries to decide about Ana's task.
    let res = d
        .post(
            &d.bia,
            CLOSE,
            &format!(
                "semana={WEEK}&destino-{}=levar&destino-{}=levar",
                s.blocked, s.bias
            ),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // Bad values.
    for form in [
        format!("semana={WEEK}&destino-{}=talvez", s.bias),
        format!("semana={WEEK}&destino-9999=levar"),
        format!("semana={WEEK}&todas=tudo"),
        "todas=levar".to_owned(),
    ] {
        let res = d.post(&d.bia, CLOSE, &form).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{form}");
    }
    let res = d.post(&d.bia, CLOSE, "semana=2030-01-01&todas=levar").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_week_closings").await, 0);

    // Closing only ever touches your own side: Ana's tasks stay put.
    let res = d
        .post(&d.bia, CLOSE, &format!("semana={WEEK}&todas=levar"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let carried: Vec<String> =
        sqlx::query_scalar("SELECT owner FROM ritmo_tasks WHERE carried_from IS NOT NULL")
            .fetch_all(&d.pool)
            .await
            .unwrap();
    assert_eq!(carried, ["bia"]);
    let closings: Vec<String> = sqlx::query_scalar("SELECT login FROM ritmo_week_closings")
        .fetch_all(&d.pool)
        .await
        .unwrap();
    assert_eq!(closings, ["bia"]);
}

#[tokio::test]
async fn closed_week_is_read_only_for_its_owner() {
    let s = semana().await;
    let d = &s.d;
    d.clock.set(MEETING);
    d.post(&d.ana, CLOSE, &format!("semana={WEEK}&todas=descartar"))
        .await;

    let forbidden = [
        (
            format!("/ritmo/tarefas/{}/mover", s.blocked),
            "para=done".to_owned(),
        ),
        (
            format!("/ritmo/tarefas/{}/apagar", s.untouched),
            String::new(),
        ),
        (
            "/ritmo/tarefas".to_owned(),
            format!("semana={WEEK}&projeto=1&titulo=Nova&tamanho=P"),
        ),
        (
            "/ritmo/semana/confirmar".to_owned(),
            format!("semana={WEEK}"),
        ),
    ];
    for (path, form) in &forbidden {
        let res = d.post(&d.ana, path, form).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{path}");
        assert!(body_text(res).await.contains("já fechou"), "{path}");
    }

    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Semana fechada 27/10"));
    assert!(body.contains("prometido 1/3 (33%)"));
    assert!(!body.contains(r#"id="nova-tarefa""#));
    let lane = |login: &str| {
        body.find(&format!(r#"<section class="raia" data-login="{login}">"#))
            .unwrap()
    };
    let ana = &body[lane("ana")..lane("bia")];
    assert!(ana.contains("Travada"));
    assert!(!ana.contains("Mover para"));
    // Bia has not closed yet: her side is still open on the meeting day.
    let res = d
        .post(
            &d.bia,
            &format!("/ritmo/tarefas/{}/mover", s.bias),
            "para=done",
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    // The check-in still works (it keeps the streak), without the frozen tasks.
    let body = d.page(&d.ana, "/ritmo/checkin").await;
    assert!(body.contains("Nenhuma tarefa ativa"));
    let res = d
        .post(
            &d.ana,
            "/ritmo/checkin",
            &format!("status-{}=advanced", s.blocked),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = d.post(&d.ana, "/ritmo/checkin", "").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn late_closing_carries_into_the_current_week() {
    let s = semana().await;
    let d = &s.d;
    // The meeting went by without closing; Thursday 29/10 Ana is reminded.
    d.clock.set(MEETING + 2 * DAY);
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("A semana anterior ainda não foi fechada"));
    assert!(body.contains(&format!(r#"href="{CLOSE}?semana={WEEK}">fechar agora"#)));
    // Bia had tasks too, but the reminder is per person.
    let body = d.page(&d.bia, "/ritmo").await;
    assert!(body.contains("A semana anterior ainda não foi fechada"));

    // The old week's board offers the button in Ana's lane.
    let body = d.page(&d.ana, &format!("/ritmo?semana={WEEK}")).await;
    assert!(body.contains(&format!(r#"href="{CLOSE}?semana={WEEK}">Fechar semana"#)));

    let res = d
        .post(&d.ana, CLOSE, &format!("semana={WEEK}&todas=levar"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["hx-redirect"], "/ritmo?semana=2026-10-28");
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(!body.contains("A semana anterior ainda não foi fechada"));
    assert_eq!(column_of(&body, "ana", "Parada").as_deref(), Some("todo"));
    assert_eq!(column_of(&body, "ana", "Travada").as_deref(), Some("doing"));
}

#[tokio::test]
async fn closing_everything_done_needs_no_decisions() {
    let d = dupla(WED_NOON).await;
    let p = d.project(&d.ana, "Livro").await;
    let id = d.task(&d.ana, WEEK, p, "Única").await;
    d.post(&d.ana, &format!("/ritmo/tarefas/{id}/mover"), "para=done")
        .await;
    d.clock.set(MEETING);
    let body = d.page(&d.ana, &format!("{CLOSE}?semana={WEEK}")).await;
    assert!(body.contains("Tudo feito"));
    // Nothing was promised: no percentage.
    assert!(body.contains("<dt>Feito do prometido</dt><dd>0</dd>"));
    let res = d.post(&d.ana, CLOSE, &format!("semana={WEEK}")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()["hx-redirect"],
        format!("/ritmo?semana={WEEK}")
    );
}
