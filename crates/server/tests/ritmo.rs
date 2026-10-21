//! Ritmo: quadro da semana, compromisso, check-in e feed da dupla, com o
//! relógio do servidor fixo em datas conhecidas.

mod common;

use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use common::{
    body_text, get, location,
    ritmo::{DAY, HOUR, TUE_NOON, WED_NOON, column_of, dupla},
};
use futures_util::StreamExt;

fn status(res: &Response) -> StatusCode {
    res.status()
}

#[tokio::test]
async fn ritmo_needs_login_and_sse_answers_401() {
    let d = dupla(WED_NOON).await;
    let res = get(&d.app, "/ritmo", None).await;
    assert_eq!(location(&res), "/login");
    let res = get(&d.app, "/ritmo/events", None).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn home_links_to_ritmo() {
    let d = dupla(WED_NOON).await;
    let body = d.page(&d.ana, "/").await;
    assert!(body.contains(r#"href="/ritmo""#));
    assert!(!body.contains("Em breve"));
}

#[tokio::test]
async fn empty_board_shows_week_lanes_and_hints() {
    let d = dupla(WED_NOON).await;
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Semana 21/10 – 27/10"));
    assert!(body.contains(r#"data-login="ana""#));
    assert!(body.contains(r#"data-login="bia""#));
    // Quem está logado vem primeiro.
    assert!(body.find(r#"data-login="ana""#) < body.find(r#"data-login="bia""#));
    assert!(body.contains("Crie um projeto"));
    assert!(body.contains("Ninguém fez check-in hoje"));
    assert!(body.contains(">Check-in de hoje<"));
    assert!(body.contains(r#"<option value="tuesday" selected>terça-feira</option>"#));
    assert!(body.contains("/static/ritmo.js"));
    // Sem semana anterior; a próxima começa depois da reunião de terça.
    assert!(!body.contains(r#"rel="prev""#));
    assert!(body.contains(r#"href="/ritmo?semana=2026-10-28" rel="next""#));
    // Não é dia de reunião.
    assert!(!body.contains("Hoje é dia de reunião"));
}

#[tokio::test]
async fn mutations_require_hx_request_header() {
    let d = dupla(WED_NOON).await;
    let req = Request::post("/ritmo/projetos")
        .header(header::COOKIE, &d.ana)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("nome=x&cor=%23000000"))
        .unwrap();
    let res = common::send(&d.app, req).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_projects").await, 0);
}

#[tokio::test]
async fn projects_are_validated_and_unique_per_owner() {
    let d = dupla(WED_NOON).await;
    d.project(&d.ana, "Livro").await;
    // Mesmo nome para outra pessoa é permitido.
    d.project(&d.bia, "Livro").await;

    let res = d
        .post(&d.ana, "/ritmo/projetos", "nome=Livro&cor=%23000000")
        .await;
    assert_eq!(status(&res), StatusCode::CONFLICT);
    assert!(body_text(res).await.contains("já tem um projeto"));

    for form in [
        "nome=+++&cor=%23000000",
        "nome=Outro&cor=vermelho",
        &format!("nome={}&cor=%23000000", "x".repeat(41)),
    ] {
        let res = d.post(&d.ana, "/ritmo/projetos", form).await;
        assert_eq!(status(&res), StatusCode::BAD_REQUEST, "{form}");
    }

    // Cor guardada normalizada; projetos aparecem no filtro.
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains(r##"data-cor="#ff8800""##));
    assert!(body.contains(r#"title="projeto de bia""#));
    assert!(body.contains(r#"<option value="1">Livro</option>"#));
    assert!(!body.contains("Crie um projeto"));
}

#[tokio::test]
async fn create_task_and_move_between_columns() {
    let d = dupla(WED_NOON).await;
    let p = d.project(&d.ana, "Livro").await;
    let res = d
        .post(
            &d.ana,
            "/ritmo/tarefas",
            &format!("semana=2026-10-21&projeto={p}&titulo=Escrever+cap%C3%ADtulo+1&tamanho=G"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["hx-trigger"], "ritmo-changed");

    let body = d.page(&d.ana, "/ritmo").await;
    let title = "Escrever capítulo 1";
    assert_eq!(column_of(&body, "ana", title).as_deref(), Some("todo"));
    assert!(body.contains(r#"<span class="tamanho" title="tamanho">G</span>"#));
    assert!(body.contains(r#"aria-label="Mover para Fazendo""#));

    let id: i64 = d.count("SELECT id FROM ritmo_tasks").await;
    let mover = format!("/ritmo/tarefas/{id}/mover");
    let res = d.post(&d.ana, &mover, "para=doing").await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(column_of(&body, "ana", title).as_deref(), Some("doing"));

    // Feito marca done_at; voltar limpa.
    d.post(&d.ana, &mover, "para=done").await;
    assert_eq!(
        d.count("SELECT COUNT(*) FROM ritmo_tasks WHERE done_at IS NOT NULL")
            .await,
        1
    );
    // Mover para a mesma coluna não muda nada.
    assert_eq!(
        d.post(&d.ana, &mover, "para=done").await.status(),
        StatusCode::OK
    );
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(column_of(&body, "ana", title).as_deref(), Some("done"));
    assert!(body.contains(r#"aria-label="Mover para Fazendo""#));

    d.post(&d.ana, &mover, "para=todo").await;
    assert_eq!(
        d.count("SELECT COUNT(*) FROM ritmo_tasks WHERE done_at IS NOT NULL")
            .await,
        0
    );

    let res = d.post(&d.ana, &mover, "para=fazendo").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Apagar.
    let res = d
        .post(&d.ana, &format!("/ritmo/tarefas/{id}/apagar"), "")
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(column_of(&body, "ana", title), None);
}

#[tokio::test]
async fn task_input_is_validated() {
    let d = dupla(WED_NOON).await;
    let p = d.project(&d.ana, "Livro").await;
    let q = d.project(&d.bia, "Corrida").await;
    let cases = [
        (
            format!("semana=2026-10-21&projeto={p}&titulo=x&tamanho=XL"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("semana=2026-10-21&projeto={p}&titulo=++&tamanho=P"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!(
                "semana=2026-10-21&projeto={p}&titulo={}&tamanho=P",
                "a".repeat(121)
            ),
            StatusCode::BAD_REQUEST,
        ),
        (
            "semana=2026-10-21&projeto=999&titulo=x&tamanho=P".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("semana=2026-10-21&projeto={q}&titulo=x&tamanho=P"),
            StatusCode::FORBIDDEN,
        ),
        (
            format!("semana=2030-01-01&projeto={p}&titulo=x&tamanho=P"),
            StatusCode::NOT_FOUND,
        ),
        (
            format!("semana=lixo&projeto={p}&titulo=x&tamanho=P"),
            StatusCode::NOT_FOUND,
        ),
        (
            "semana=2026-10-21&projeto=abc&titulo=x&tamanho=P".to_owned(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];
    for (form, expected) in cases {
        let res = d.post(&d.ana, "/ritmo/tarefas", &form).await;
        assert_eq!(res.status(), expected, "{form}");
    }
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_tasks").await, 0);

    // Título com quebra de linha e espaços sobrando vira uma linha só.
    let res = d
        .post(
            &d.ana,
            "/ritmo/tarefas",
            &format!("semana=2026-10-21&projeto={p}&titulo=+Ler%0A%0Amais+&tamanho=P"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        d.count("SELECT COUNT(*) FROM ritmo_tasks WHERE title = 'Ler mais'")
            .await,
        1
    );
}

#[tokio::test]
async fn cannot_edit_the_other_users_task() {
    let d = dupla(WED_NOON).await;
    let p = d.project(&d.ana, "Livro").await;
    let id = d.task(&d.ana, "2026-10-21", p, "Revisar").await;

    let res = d
        .post(&d.bia, &format!("/ritmo/tarefas/{id}/mover"), "para=doing")
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(body_text(res).await.contains("Só quem criou"));
    let res = d
        .post(&d.bia, &format!("/ritmo/tarefas/{id}/apagar"), "")
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_tasks").await, 1);

    let res = d
        .post(&d.bia, "/ritmo/tarefas/999/mover", "para=doing")
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Bia vê a tarefa da Ana, mas sem botões de mexer nela.
    let body = d.page(&d.bia, "/ritmo").await;
    assert_eq!(column_of(&body, "ana", "Revisar").as_deref(), Some("todo"));
    assert!(!body.contains(&format!("/ritmo/tarefas/{id}/mover")));
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains(&format!("/ritmo/tarefas/{id}/mover")));
}

#[tokio::test]
async fn confirm_week_freezes_the_promise() {
    let d = dupla(WED_NOON).await;
    let confirm = "/ritmo/semana/confirmar";

    let res = d.post(&d.ana, confirm, "semana=2026-10-21").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "sem tarefas");

    let p = d.project(&d.ana, "Livro").await;
    let a = d.task(&d.ana, "2026-10-21", p, "Tarefa+A").await;
    let b = d.task(&d.ana, "2026-10-21", p, "Tarefa+B").await;
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Confirmar semana"));

    let res = d.post(&d.ana, confirm, "semana=2026-10-21").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["hx-trigger"], "ritmo-changed");
    let res = d.post(&d.ana, confirm, "semana=2026-10-21").await;
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // Depois de confirmar: tarefa extra não entra no prometido.
    d.task(&d.ana, "2026-10-21", p, "Extra").await;
    d.post(&d.ana, &format!("/ritmo/tarefas/{a}/mover"), "para=done")
        .await;
    let body = d.page(&d.bia, "/ritmo").await;
    assert!(body.contains("Prometido: 1/2 feito · confirmado 21/10 12:00"));
    assert_eq!(
        body.matches(r#"<span class="selo">prometido</span>"#)
            .count(),
        2
    );
    // Bia não confirmou nem tem tarefas.
    assert!(body.contains("Semana não confirmada"));
    assert!(!body.contains("Confirmar semana"));

    // Apagar uma prometida não apaga o que foi prometido.
    d.post(&d.ana, &format!("/ritmo/tarefas/{b}/apagar"), "")
        .await;
    assert_eq!(
        d.count("SELECT COUNT(*) FROM ritmo_commitment_tasks").await,
        2
    );
    assert_eq!(
        d.count("SELECT COUNT(*) FROM ritmo_commitment_tasks WHERE task_id IS NULL")
            .await,
        1
    );
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Prometido: 1/2 feito"));

    let res = d.post(&d.ana, confirm, "semana=1999-01-01").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn checkin_once_per_day_and_editable() {
    let d = dupla(WED_NOON).await;

    let body = d.page(&d.ana, "/ritmo/checkin").await;
    assert!(body.contains("Nenhuma tarefa ativa"));

    let p = d.project(&d.ana, "Livro").await;
    let a = d.task(&d.ana, "2026-10-21", p, "Escrever").await;
    let b = d.task(&d.ana, "2026-10-21", p, "Ler").await;
    let pronta = d.task(&d.ana, "2026-10-21", p, "Pronta").await;
    d.post(
        &d.ana,
        &format!("/ritmo/tarefas/{pronta}/mover"),
        "para=done",
    )
    .await;

    let body = d.page(&d.ana, "/ritmo/checkin").await;
    assert!(body.contains("21/10 (quarta-feira)"));
    assert!(body.contains(&format!(r#"name="status-{a}" value="untouched" checked"#)));
    assert!(body.contains(&format!(r#"name="nota-{b}""#)));
    assert!(
        !body.contains(&format!("status-{pronta}")),
        "feita não é ativa"
    );
    assert!(body.contains("Salvar check-in"));

    let res = d
        .post(
            &d.ana,
            "/ritmo/checkin",
            &format!(
                "status-{a}=advanced&nota-{a}=Duas+p%C3%A1ginas&status-{b}=untouched&nota-{b}="
            ),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["hx-redirect"], "/ritmo#celebrar");
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_checkins").await, 1);

    // Mesmo dia: atualiza em vez de criar outro.
    d.clock.set(WED_NOON + 3 * HOUR);
    let body = d.page(&d.ana, "/ritmo/checkin").await;
    assert!(body.contains("Atualizar check-in"));
    assert!(body.contains(r#"value="Duas páginas""#));
    assert!(body.contains(&format!(r#"name="status-{a}" value="advanced" checked"#)));
    let res = d
        .post(
            &d.ana,
            "/ritmo/checkin",
            &format!("status-{a}=blocked&nota-{a}=Sem+ideia&status-{b}=untouched"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_checkins").await, 1);
    assert_eq!(
        d.count("SELECT COUNT(*) FROM ritmo_checkin_items WHERE status = 'blocked'")
            .await,
        1
    );
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Editar check-in de hoje"));
    assert!(body.contains("<small>às 15:00</small>"));

    // Tarefa que foi para Feito depois do check-in continua editável hoje.
    d.post(&d.ana, &format!("/ritmo/tarefas/{a}/mover"), "para=done")
        .await;
    let body = d.page(&d.ana, "/ritmo/checkin").await;
    assert!(body.contains(&format!(r#"name="status-{a}" value="blocked" checked"#)));

    // Dia seguinte: novo check-in.
    d.clock.set(WED_NOON + DAY);
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains(">Check-in de hoje<"));
    let body = d.page(&d.ana, "/ritmo/checkin").await;
    assert!(!body.contains(&format!("status-{a}")));
    d.post(&d.ana, "/ritmo/checkin", &format!("status-{b}=advanced"))
        .await;
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_checkins").await, 2);
}

#[tokio::test]
async fn checkin_input_is_validated() {
    let d = dupla(WED_NOON).await;
    let p = d.project(&d.ana, "Livro").await;
    let a = d.task(&d.ana, "2026-10-21", p, "Escrever").await;
    let q = d.project(&d.bia, "Corrida").await;
    let other = d.task(&d.bia, "2026-10-21", q, "Correr").await;

    let cases = [
        (format!("status-{a}=talvez"), StatusCode::BAD_REQUEST),
        ("status-abc=advanced".to_owned(), StatusCode::BAD_REQUEST),
        (format!("status-{other}=advanced"), StatusCode::FORBIDDEN),
        (format!("nota-{other}=oi"), StatusCode::FORBIDDEN),
        (
            format!("status-{a}=advanced&nota-{a}={}", "x".repeat(141)),
            StatusCode::BAD_REQUEST,
        ),
    ];
    for (form, expected) in cases {
        let res = d.post(&d.ana, "/ritmo/checkin", &form).await;
        assert_eq!(res.status(), expected, "{form}");
    }
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_checkins").await, 0);

    // Campos desconhecidos são ignorados; check-in sem itens também vale.
    let res = d.post(&d.ana, "/ritmo/checkin", "outro=1").await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = d.page(&d.bia, "/ritmo").await;
    assert!(body.contains("Check-in sem tarefas."));
}

#[tokio::test]
async fn partner_sees_my_checkin_in_the_feed() {
    let d = dupla(WED_NOON).await;
    let p = d.project(&d.ana, "Livro").await;
    let a = d.task(&d.ana, "2026-10-21", p, "Escrever").await;
    d.post(
        &d.ana,
        "/ritmo/checkin",
        &format!("status-{a}=blocked&nota-{a}=Travei+no+fim"),
    )
    .await;

    let body = d.page(&d.bia, "/ritmo").await;
    let feed = &body[body.find(r#"<section class="feed""#).unwrap()..];
    assert!(feed.contains(r#"<article class="checkin-feito" data-login="ana">"#));
    assert!(feed.contains("<h4>ana <small>às 12:00</small></h4>"));
    assert!(feed.contains(r#"<span class="status" data-status="blocked">travei</span>"#));
    assert!(feed.contains("— Travei no fim"));
    // Bia ainda não fez o dela.
    assert!(body.contains(">Check-in de hoje<"));

    // A Ana vê o próprio check-in como "Você".
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("<h4>Você <small>"));

    // O parcial (usado pelo SSE) traz o mesmo feed, sem a página em volta.
    let body = d.page(&d.bia, "/ritmo/quadro").await;
    assert!(body.starts_with(r#"<section id="quadro""#));
    assert!(!body.contains("<html"));
    assert!(body.contains("Travei no fim"));

    // No dia seguinte o feed zera.
    d.clock.set(WED_NOON + DAY);
    let body = d.page(&d.bia, "/ritmo").await;
    assert!(body.contains("Ninguém fez check-in hoje"));
}

#[tokio::test]
async fn week_turns_at_the_end_of_tuesday() {
    // Terça 23:30 em Brasília (quarta 02:30 UTC): ainda é a semana que termina hoje.
    let d = dupla(TUE_NOON + 11 * HOUR + 30 * 60).await;
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Semana 14/10 – 20/10"));
    assert!(body.contains("Hoje é dia de reunião"));
    assert!(body.contains(r#"href="/ritmo?semana=2026-10-21">planejar a próxima</a>"#));
    assert!(body.contains(r#"href="/ritmo/semana/fechar?semana=2026-10-14">fechar a semana"#));

    // Na reunião, planeja a próxima semana.
    let p = d.project(&d.ana, "Livro").await;
    let old = d.task(&d.ana, "2026-10-14", p, "Antiga").await;
    let new = d.task(&d.ana, "2026-10-21", p, "Nova").await;
    let body = d.page(&d.ana, "/ritmo?semana=2026-10-21").await;
    assert!(body.contains("Semana 21/10 – 27/10"));
    assert!(body.contains("Planejando a próxima semana"));
    assert!(body.contains(r#"href="/ritmo?semana=2026-10-14" rel="prev""#));
    assert!(!body.contains(r#"rel="next""#));
    assert_eq!(column_of(&body, "ana", "Nova").as_deref(), Some("todo"));
    assert_eq!(column_of(&body, "ana", "Antiga"), None);
    let res = d
        .post(&d.ana, "/ritmo/semana/confirmar", "semana=2026-10-21")
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    // Duas semanas à frente não existe.
    let res = get(&d.app, "/ritmo?semana=2026-10-28", Some(&d.ana)).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Uma hora depois (quarta 00:30): virou a semana.
    d.clock.set(TUE_NOON + 12 * HOUR + 30 * 60);
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Semana 21/10 – 27/10"));
    assert!(!body.contains("Hoje é dia de reunião"));
    assert_eq!(column_of(&body, "ana", "Nova").as_deref(), Some("todo"));
    assert!(body.contains("Prometido: 0/1 feito"));
    assert!(body.contains(r#"href="/ritmo?semana=2026-10-28" rel="next""#));

    // A semana passada fica só para leitura.
    let body = d.page(&d.ana, "/ritmo?semana=2026-10-14").await;
    assert!(body.contains("Semana encerrada"));
    assert!(body.contains(r#"href="/ritmo?semana=2026-10-21" rel="next""#));
    assert!(!body.contains("nova-tarefa"));
    assert!(!body.contains(&format!("/ritmo/tarefas/{old}/mover")));
    let res = d
        .post(&d.ana, &format!("/ritmo/tarefas/{old}/mover"), "para=done")
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(body_text(res).await.contains("terminou"));
    let res = d
        .post(
            &d.ana,
            "/ritmo/tarefas",
            &format!("semana=2026-10-14&projeto={p}&titulo=Tarde&tamanho=P"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = d
        .post(&d.ana, "/ritmo/semana/confirmar", "semana=2026-10-14")
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Check-in de hoje só olha a semana atual.
    let body = d.page(&d.ana, "/ritmo/checkin").await;
    assert!(body.contains(&format!("status-{new}")));
    assert!(!body.contains(&format!("status-{old}")));
    let res = d
        .post(&d.ana, "/ritmo/checkin", &format!("status-{old}=advanced"))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    for bad in ["/ritmo?semana=lixo", "/ritmo?semana=2026-10-07"] {
        let res = get(&d.app, bad, Some(&d.ana)).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{bad}");
    }
}

#[tokio::test]
async fn changing_the_meeting_day_only_affects_new_weeks() {
    let d = dupla(WED_NOON).await;
    // Cria a semana atual (21/10 – 27/10).
    d.page(&d.ana, "/ritmo").await;

    let res = d.post(&d.bia, "/ritmo/config", "dia=thursday").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["hx-refresh"], "true");
    let res = d.post(&d.bia, "/ritmo/config", "dia=quinta").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Semana 21/10 – 27/10"));
    assert!(body.contains(r#"<option value="thursday" selected>quinta-feira</option>"#));
    assert!(body.contains("A semana vira no fim de quinta-feira"));

    // Semana de transição: de quarta 28/10 até a quinta 29/10.
    d.clock.set(WED_NOON + 7 * DAY);
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Semana 28/10 – 29/10"));
    // Depois disso, semanas de sexta a quinta.
    d.clock.set(WED_NOON + 9 * DAY);
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Semana 30/10 – 05/11"));
}

#[tokio::test]
async fn filter_by_project() {
    let d = dupla(WED_NOON).await;
    let livro = d.project(&d.ana, "Livro").await;
    let app = d.project(&d.ana, "App").await;
    let corrida = d.project(&d.bia, "Corrida").await;
    d.task(&d.ana, "2026-10-21", livro, "Escrever").await;
    d.task(&d.ana, "2026-10-21", app, "Programar").await;
    d.task(&d.bia, "2026-10-21", corrida, "Correr").await;

    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains(r#"<a href="/ritmo?semana=2026-10-21" aria-current="true">Todos</a>"#));
    for title in ["Escrever", "Programar", "Correr"] {
        assert!(body.contains(title));
    }

    let url = format!("/ritmo?semana=2026-10-21&projeto={app}");
    let body = d.page(&d.ana, &url).await;
    assert!(body.contains("Programar"));
    assert!(!body.contains(r#"<span class="titulo">Escrever</span>"#));
    assert!(!body.contains(r#"<span class="titulo">Correr</span>"#));
    assert!(body.contains(&format!(
        r#"hx-get="/ritmo/quadro?semana=2026-10-21&#38;projeto={app}""#
    )));
    assert!(body.contains(r#"aria-current="true">App</a>"#));

    let body = d
        .page(
            &d.bia,
            &format!("/ritmo/quadro?semana=2026-10-21&projeto={corrida}"),
        )
        .await;
    assert!(body.contains("Correr"));
    assert!(!body.contains(r#"<span class="titulo">Programar</span>"#));

    let res = get(&d.app, "/ritmo?projeto=999", Some(&d.ana)).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bad_settings_fall_back_to_defaults() {
    let d = dupla(TUE_NOON).await;
    sqlx::query("UPDATE ritmo_settings SET value = 'lixo'")
        .execute(&d.pool)
        .await
        .unwrap();
    let body = d.page(&d.ana, "/ritmo").await;
    // Terça em Brasília, reunião na terça.
    assert!(body.contains("Semana 14/10 – 20/10"));
    assert!(body.contains("Hoje é dia de reunião"));

    // Fuso configurado: UTC+9 já é quarta.
    sqlx::query(
        "UPDATE ritmo_settings SET value = CASE key WHEN 'meeting_day' THEN 'tuesday' ELSE '540' END",
    )
    .execute(&d.pool)
    .await
    .unwrap();
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains("Semana 21/10 – 27/10"));
}

#[tokio::test]
async fn events_stream_announces_changes() {
    let d = dupla(WED_NOON).await;
    let res = get(&d.app, "/ritmo/events", Some(&d.bia)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "text/event-stream");
    let mut stream = res.into_body().into_data_stream();

    d.project(&d.ana, "Livro").await;
    let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("evento em até 5 s")
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&frame);
    assert!(text.contains("event: ritmo"), "{text}");
    assert!(text.contains("data: mudou"), "{text}");
}

#[tokio::test]
async fn many_changes_without_listeners_do_not_fail() {
    let d = dupla(WED_NOON).await;
    let res = get(&d.app, "/ritmo/events", Some(&d.bia)).await;
    let mut stream = res.into_body().into_data_stream();
    // Mais avisos do que cabem no canal: quem atrasou recebe um aviso só.
    let p = d.project(&d.ana, "Livro").await;
    for i in 0..40 {
        d.task(&d.ana, "2026-10-21", p, &format!("T{i}")).await;
    }
    let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&frame).contains("event: ritmo"));
}
