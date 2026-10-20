//! Ritmo: Kanban semanal com compromisso ("prometido") e check-in diário.
//!
//! O servidor é a fonte da verdade e as páginas recebem HTML pronto (HTMX).
//! Toda mutação avisa as páginas abertas por SSE (`/ritmo/events`), que
//! buscam de novo o parcial do quadro. Cada um só altera o que é seu, mas vê
//! tudo da dupla.

pub mod date;
mod db;

use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
};

use askama::Template;
use axum::{
    Extension, Form, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::Stream;
use serde::Deserialize;
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::{AppState, auth::CurrentUser, pages::render};
use date::{Date, Weekday, local_time, plan_week};

const DEFAULT_MEETING: Weekday = Weekday::Tuesday;
/// Brasília (UTC-3, sem horário de verão).
const DEFAULT_OFFSET_MINUTES: i64 = -180;
const MAX_TITLE: usize = 120;
const MAX_PROJECT: usize = 40;
const MAX_NOTE: usize = 140;

/// Colunas do quadro: valor no banco e rótulo.
const COLUMNS: [(&str, &str); 3] = [("todo", "A fazer"), ("doing", "Fazendo"), ("done", "Feito")];
/// Respostas do check-in: valor no banco e rótulo.
const CHECKIN: [(&str, &str); 3] = [
    ("advanced", "avancei"),
    ("blocked", "travei"),
    ("untouched", "não mexi"),
];
const SIZES: [&str; 3] = ["P", "M", "G"];
/// Evento HTMX que faz o quadro se recarregar.
const CHANGED: &str = "ritmo-changed";

pub fn routes() -> Router<AppState> {
    let mutations = Router::new()
        .route("/ritmo/projetos", post(create_project))
        .route("/ritmo/tarefas", post(create_task))
        .route("/ritmo/tarefas/{id}/mover", post(move_task))
        .route("/ritmo/tarefas/{id}/apagar", post(delete_task))
        .route("/ritmo/semana/confirmar", post(confirm_week))
        .route("/ritmo/checkin", post(save_checkin))
        .route("/ritmo/config", post(save_config))
        .route_layer(middleware::from_fn(require_htmx));
    Router::new()
        .route("/ritmo", get(page))
        .route("/ritmo/quadro", get(board_partial))
        .route("/ritmo/checkin", get(checkin_page))
        .route("/ritmo/events", get(events))
        .merge(mutations)
}

/// Aviso de "algo mudou no Ritmo" para as páginas abertas.
#[derive(Clone)]
pub struct Events(broadcast::Sender<()>);

impl Default for Events {
    fn default() -> Self {
        Self(broadcast::channel(16).0)
    }
}

impl Events {
    pub fn notify(&self) {
        // Sem ninguém ouvindo, `send` falha; tudo bem.
        let _ = self.0.send(());
    }
}

#[derive(Debug)]
enum Erro {
    NaoEncontrado,
    Proibido(&'static str),
    Invalido(&'static str),
    Conflito(&'static str),
    Interno(sqlx::Error),
}

impl From<sqlx::Error> for Erro {
    fn from(e: sqlx::Error) -> Self {
        Self::Interno(e)
    }
}

impl IntoResponse for Erro {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            Self::NaoEncontrado => (StatusCode::NOT_FOUND, "Não encontrado."),
            Self::Proibido(m) => (StatusCode::FORBIDDEN, m),
            Self::Invalido(m) => (StatusCode::BAD_REQUEST, m),
            Self::Conflito(m) => (StatusCode::CONFLICT, m),
            Self::Interno(e) => {
                tracing::error!("erro no Ritmo: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Erro interno. Tente de novo.",
                )
            }
        };
        (status, msg).into_response()
    }
}

type Resultado = Result<Response, Erro>;

/// Proteção extra contra CSRF (além do cookie `SameSite=Lax`): mutações só
/// pelo HTMX, que manda `HX-Request: true`. Formulários de outros sites não
/// conseguem pôr esse cabeçalho.
async fn require_htmx(req: Request, next: Next) -> Response {
    if req.headers().get("hx-request").is_some_and(|v| v == "true") {
        next.run(req).await
    } else {
        Erro::Proibido("Requisição sem HX-Request.").into_response()
    }
}

/// Resposta das mutações: avisa todos por SSE e dispara a recarga do quadro
/// na própria página.
fn changed(state: &AppState) -> Response {
    state.ritmo.notify();
    (
        [(
            HeaderName::from_static("hx-trigger"),
            HeaderValue::from_static(CHANGED),
        )],
        StatusCode::OK,
    )
        .into_response()
}

/// Para mudanças que afetam formulários fora do quadro (projetos, config).
fn changed_refresh(state: &AppState) -> Response {
    state.ritmo.notify();
    (
        [(
            HeaderName::from_static("hx-refresh"),
            HeaderValue::from_static("true"),
        )],
        StatusCode::OK,
    )
        .into_response()
}

#[derive(Debug, Clone)]
struct Week {
    id: i64,
    start: Date,
    end: Date,
}

impl TryFrom<db::WeekRow> for Week {
    type Error = sqlx::Error;

    fn try_from(row: db::WeekRow) -> Result<Self, sqlx::Error> {
        let parse = |s: &str| {
            Date::parse(s).ok_or_else(|| sqlx::Error::Protocol(format!("data inválida: {s}")))
        };
        Ok(Self {
            id: row.id,
            start: parse(&row.start_date)?,
            end: parse(&row.end_date)?,
        })
    }
}

fn to_week(row: Option<db::WeekRow>) -> sqlx::Result<Option<Week>> {
    row.map(Week::try_from).transpose()
}

/// O "agora" da requisição: configuração, hoje e a semana atual.
struct Ctx {
    meeting: Weekday,
    offset: i64,
    now: i64,
    today: Date,
    current: Week,
}

impl Ctx {
    fn editable(&self, week: &Week) -> bool {
        week.end >= self.today
    }

    fn stamp(&self, secs: i64) -> String {
        format!(
            "{} {}",
            Date::from_unix(secs, self.offset).short(),
            local_time(secs, self.offset)
        )
    }
}

async fn context(state: &AppState) -> Result<Ctx, Erro> {
    let meeting = db::setting(&state.db, "meeting_day")
        .await?
        .as_deref()
        .and_then(Weekday::from_key)
        .unwrap_or(DEFAULT_MEETING);
    let offset = db::setting(&state.db, "utc_offset_minutes")
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|o| o.abs() <= 14 * 60)
        .unwrap_or(DEFAULT_OFFSET_MINUTES);
    let now = state.clock.now();
    let today = Date::from_unix(now, offset);
    let current = ensure_week(&state.db, today, meeting).await?;
    Ok(Ctx {
        meeting,
        offset,
        now,
        today,
        current,
    })
}

/// Semana que contém `day`, criada se preciso sem sobrepor as vizinhas.
async fn ensure_week(db: &SqlitePool, day: Date, meeting: Weekday) -> sqlx::Result<Week> {
    let key = day.to_string();
    if let Some(week) = to_week(db::week_containing(db, &key).await?)? {
        return Ok(week);
    }
    let prev = to_week(db::week_before(db, &key).await?)?.map(|w| w.end);
    let next = to_week(db::week_after(db, &key).await?)?.map(|w| w.start);
    let (start, end) = plan_week(day, meeting, prev, next);
    db::insert_week(db, &start.to_string(), &end.to_string()).await?;
    to_week(db::week_containing(db, &key).await?)?
        .ok_or_else(|| sqlx::Error::Protocol(format!("semana de {key} não foi criada")))
}

/// Semana pedida em `?semana=<início>`: a atual, a próxima (criada se
/// preciso) ou uma passada que já exista.
async fn resolve_week(state: &AppState, ctx: &Ctx, semana: Option<&str>) -> Result<Week, Erro> {
    let Some(semana) = semana else {
        return Ok(ctx.current.clone());
    };
    let start = Date::parse(semana).ok_or(Erro::NaoEncontrado)?;
    if start == ctx.current.start {
        return Ok(ctx.current.clone());
    }
    if start == ctx.current.end.add_days(1) {
        return Ok(ensure_week(&state.db, start, ctx.meeting).await?);
    }
    to_week(db::week_by_start(&state.db, semana).await?)?.ok_or(Erro::NaoEncontrado)
}

/// Raias: quem está logado primeiro, depois os outros convidados.
fn people(state: &AppState, me: &str) -> Vec<String> {
    std::iter::once(me.to_owned())
        .chain(state.auth.allowed.iter().filter(|a| *a != me).cloned())
        .collect()
}

fn column(key: &str) -> Option<&'static str> {
    COLUMNS.iter().map(|(k, _)| *k).find(|k| *k == key)
}

/// Uma linha só, sem caracteres de controle e com espaços normalizados.
fn one_line(s: &str, max: usize) -> Result<String, Erro> {
    let clean: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if clean.chars().count() > max {
        return Err(Erro::Invalido("Texto longo demais."));
    }
    Ok(clean)
}

fn color(s: &str) -> Option<String> {
    let s = s.trim();
    let hex = s.strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

// ---------- Visões ----------

struct TaskView {
    id: i64,
    title: String,
    size: String,
    project_name: String,
    project_color: String,
    promised: bool,
    prev: Option<(&'static str, &'static str)>,
    next: Option<(&'static str, &'static str)>,
}

struct ColumnView {
    key: &'static str,
    label: &'static str,
    tasks: Vec<TaskView>,
}

struct LaneView {
    login: String,
    mine: bool,
    columns: Vec<ColumnView>,
    confirmed: Option<String>,
    promised_total: usize,
    promised_done: usize,
    can_confirm: bool,
}

struct FeedItemView {
    title: String,
    color: String,
    status: &'static str,
    label: &'static str,
    note: String,
}

struct FeedView {
    login: String,
    mine: bool,
    time: String,
    items: Vec<FeedItemView>,
}

struct Chip {
    url: String,
    name: String,
    color: String,
    owner: String,
    active: bool,
}

// Visão do template: as flags viram condições no HTML.
#[allow(clippy::struct_excessive_bools)]
struct Quadro {
    semana: String,
    label: String,
    editable: bool,
    is_current: bool,
    is_meeting_day: bool,
    prev: Option<String>,
    next: Option<String>,
    refresh_url: String,
    all_url: String,
    filtered: bool,
    chips: Vec<Chip>,
    lanes: Vec<LaneView>,
    show_feed: bool,
    checked_in: bool,
    feed: Vec<FeedView>,
}

fn status_label(key: &str) -> (&'static str, &'static str) {
    CHECKIN
        .iter()
        .copied()
        .find(|(k, _)| *k == key)
        .unwrap_or(CHECKIN[2])
}

/// Raias da semana: uma por pessoa, com as colunas e o resumo do prometido.
async fn build_lanes(
    state: &AppState,
    ctx: &Ctx,
    me: &str,
    week: &Week,
    logins: &[String],
    projeto: Option<i64>,
) -> Result<Vec<LaneView>, Erro> {
    let tasks = db::tasks_of_week(&state.db, week.id).await?;
    let commitments = db::commitments(&state.db, week.id).await?;
    let promised = db::promised(&state.db, week.id).await?;
    let promised_ids: HashSet<i64> = promised.iter().filter_map(|p| p.task_id).collect();
    let done_ids: HashSet<i64> = tasks
        .iter()
        .filter(|t| t.status == "done")
        .map(|t| t.id)
        .collect();
    let editable = ctx.editable(week);
    let task_view = |i: usize, t: &db::TaskRow| TaskView {
        id: t.id,
        title: t.title.clone(),
        size: t.size.clone(),
        project_name: t.project_name.clone(),
        project_color: t.project_color.clone(),
        promised: promised_ids.contains(&t.id),
        prev: i.checked_sub(1).map(|j| COLUMNS[j]),
        next: COLUMNS.get(i + 1).copied(),
    };

    Ok(logins
        .iter()
        .map(|login| {
            let columns = COLUMNS
                .iter()
                .enumerate()
                .map(|(i, (key, label))| ColumnView {
                    key,
                    label,
                    tasks: tasks
                        .iter()
                        .filter(|t| &t.owner == login && t.status == *key)
                        .filter(|t| projeto.is_none_or(|p| t.project_id == p))
                        .map(|t| task_view(i, t))
                        .collect(),
                })
                .collect();
            let mine_promised = promised.iter().filter(|p| &p.login == login);
            let confirmed = commitments
                .iter()
                .find(|c| &c.login == login)
                .map(|c| ctx.stamp(c.confirmed_at));
            LaneView {
                mine: login == me,
                can_confirm: login == me
                    && editable
                    && confirmed.is_none()
                    && tasks.iter().any(|t| t.owner == me),
                confirmed,
                promised_total: mine_promised.clone().count(),
                promised_done: mine_promised
                    .filter(|p| p.task_id.is_some_and(|id| done_ids.contains(&id)))
                    .count(),
                login: login.clone(),
                columns,
            }
        })
        .collect())
}

/// Check-ins de hoje (o do outro primeiro) e se `me` já fez o seu.
async fn build_feed(
    state: &AppState,
    ctx: &Ctx,
    me: &str,
    logins: &[String],
) -> Result<(Vec<FeedView>, bool), Erro> {
    let today = ctx.today.to_string();
    let done = db::checkins_of_day(&state.db, &today).await?;
    let mut feed = Vec::new();
    for login in logins.iter().skip(1).chain(logins.first()) {
        let Some(c) = done.iter().find(|c| &c.login == login) else {
            continue;
        };
        let items = db::checkin_items(&state.db, login, &today)
            .await?
            .into_iter()
            .map(|it| {
                let (status, label) = status_label(&it.status);
                FeedItemView {
                    title: it.title,
                    color: it.project_color,
                    status,
                    label,
                    note: it.note,
                }
            })
            .collect();
        feed.push(FeedView {
            login: login.clone(),
            mine: login == me,
            time: local_time(c.confirmed_at, ctx.offset),
            items,
        });
    }
    let checked_in = feed.iter().any(|f| f.mine);
    Ok((feed, checked_in))
}

async fn build_quadro(
    state: &AppState,
    ctx: &Ctx,
    me: &str,
    week: &Week,
    projeto: Option<i64>,
) -> Result<Quadro, Erro> {
    let semana = week.start.to_string();
    let projects = db::projects(&state.db).await?;
    if let Some(p) = projeto
        && !projects.iter().any(|x| x.id == p)
    {
        return Err(Erro::NaoEncontrado);
    }
    let editable = ctx.editable(week);
    let is_current = week.id == ctx.current.id;
    let logins = people(state, me);
    let lanes = build_lanes(state, ctx, me, week, &logins, projeto).await?;
    let (feed, checked_in) = if is_current {
        build_feed(state, ctx, me, &logins).await?
    } else {
        (Vec::new(), false)
    };

    let prev = db::week_before(&state.db, &semana)
        .await?
        .map(|w| w.start_date);
    let next = if week.start > ctx.current.start {
        None
    } else if is_current {
        Some(ctx.current.end.add_days(1).to_string())
    } else {
        db::week_after(&state.db, &semana)
            .await?
            .map(|w| w.start_date)
    };
    let all_url = format!("/ritmo?semana={semana}");
    let chips = projects
        .into_iter()
        .map(|p| Chip {
            url: format!("{all_url}&projeto={}", p.id),
            active: projeto == Some(p.id),
            name: p.name,
            color: p.color,
            owner: p.owner,
        })
        .collect();

    Ok(Quadro {
        refresh_url: format!(
            "/ritmo/quadro?semana={semana}{}",
            projeto.map(|p| format!("&projeto={p}")).unwrap_or_default()
        ),
        label: format!("{} – {}", week.start.short(), week.end.short()),
        is_meeting_day: is_current && ctx.today == week.end,
        filtered: projeto.is_some(),
        semana,
        editable,
        is_current,
        prev,
        next,
        all_url,
        chips,
        lanes,
        show_feed: is_current,
        checked_in,
        feed,
    })
}

// ---------- Páginas ----------

struct ProjectOption {
    id: i64,
    name: String,
}

struct DayOption {
    key: &'static str,
    label: &'static str,
    selected: bool,
}

#[derive(Template)]
#[template(path = "ritmo/page.html")]
struct Page {
    user: String,
    q: Quadro,
    my_projects: Vec<ProjectOption>,
    days: Vec<DayOption>,
    meeting_label: &'static str,
}

#[derive(Template)]
#[template(path = "ritmo/_quadro.html")]
struct BoardPartial {
    q: Quadro,
}

#[derive(Deserialize)]
pub struct VerQuery {
    semana: Option<String>,
    projeto: Option<i64>,
}

async fn page(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Query(query): Query<VerQuery>,
) -> Resultado {
    let ctx = context(&state).await?;
    let week = resolve_week(&state, &ctx, query.semana.as_deref()).await?;
    let q = build_quadro(&state, &ctx, &user.login, &week, query.projeto).await?;
    let my_projects = db::projects(&state.db)
        .await?
        .into_iter()
        .filter(|p| p.owner == user.login)
        .map(|p| ProjectOption {
            id: p.id,
            name: p.name,
        })
        .collect();
    let days = Weekday::ALL
        .into_iter()
        .map(|d| DayOption {
            key: d.key(),
            label: d.label(),
            selected: d == ctx.meeting,
        })
        .collect();
    Ok(render(&Page {
        user: user.login,
        q,
        my_projects,
        days,
        meeting_label: ctx.meeting.label(),
    }))
}

async fn board_partial(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Query(query): Query<VerQuery>,
) -> Resultado {
    let ctx = context(&state).await?;
    let week = resolve_week(&state, &ctx, query.semana.as_deref()).await?;
    let q = build_quadro(&state, &ctx, &user.login, &week, query.projeto).await?;
    Ok(render(&BoardPartial { q }))
}

struct OptionView {
    key: &'static str,
    label: &'static str,
    checked: bool,
}

struct CheckinItemView {
    id: i64,
    title: String,
    project: String,
    color: String,
    note: String,
    options: Vec<OptionView>,
}

#[derive(Template)]
#[template(path = "ritmo/checkin.html")]
struct CheckinPage {
    user: String,
    today: String,
    existing: bool,
    items: Vec<CheckinItemView>,
}

async fn checkin_page(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> Resultado {
    let ctx = context(&state).await?;
    let today = ctx.today.to_string();
    let previous: HashMap<i64, db::CheckinItemRow> =
        db::checkin_items(&state.db, &user.login, &today)
            .await?
            .into_iter()
            .map(|it| (it.task_id, it))
            .collect();
    let existing = !db::checkins_of_day(&state.db, &today)
        .await?
        .iter()
        .all(|c| c.login != user.login);
    // Tarefas ativas, mais as que já estavam no check-in de hoje.
    let items = db::tasks_of_week(&state.db, ctx.current.id)
        .await?
        .into_iter()
        .filter(|t| t.owner == user.login)
        .filter(|t| t.status != "done" || previous.contains_key(&t.id))
        .map(|t| {
            let before = previous.get(&t.id);
            let chosen = before.map_or("untouched", |b| b.status.as_str());
            CheckinItemView {
                id: t.id,
                options: CHECKIN
                    .iter()
                    .map(|(key, label)| OptionView {
                        key,
                        label,
                        checked: *key == chosen,
                    })
                    .collect(),
                note: before.map(|b| b.note.clone()).unwrap_or_default(),
                title: t.title,
                project: t.project_name,
                color: t.project_color,
            }
        })
        .collect();
    Ok(render(&CheckinPage {
        user: user.login,
        today: format!("{} ({})", ctx.today.short(), ctx.today.weekday().label()),
        existing,
        items,
    }))
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.ritmo.0.subscribe();
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            // Atrasado também vale como "mudou": o quadro é buscado inteiro.
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                Some((Ok(Event::default().event("ritmo").data("mudou")), rx))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---------- Mutações ----------

#[derive(Deserialize)]
pub struct NovoProjeto {
    nome: String,
    cor: String,
}

async fn create_project(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Form(form): Form<NovoProjeto>,
) -> Resultado {
    let name = one_line(&form.nome, MAX_PROJECT)?;
    if name.is_empty() {
        return Err(Erro::Invalido("Dê um nome ao projeto."));
    }
    let color = color(&form.cor).ok_or(Erro::Invalido("Cor inválida."))?;
    let now = state.clock.now();
    if !db::insert_project(&state.db, &user.login, &name, &color, now).await? {
        return Err(Erro::Conflito("Você já tem um projeto com esse nome."));
    }
    Ok(changed_refresh(&state))
}

#[derive(Deserialize)]
pub struct NovaTarefa {
    semana: String,
    projeto: i64,
    titulo: String,
    tamanho: String,
}

async fn create_task(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Form(form): Form<NovaTarefa>,
) -> Resultado {
    let ctx = context(&state).await?;
    let week = resolve_week(&state, &ctx, Some(&form.semana)).await?;
    if !ctx.editable(&week) {
        return Err(Erro::Proibido("Essa semana já terminou."));
    }
    let project = db::project(&state.db, form.projeto)
        .await?
        .ok_or(Erro::Invalido("Projeto não existe."))?;
    if project.owner != user.login {
        return Err(Erro::Proibido("Esse projeto não é seu."));
    }
    let title = one_line(&form.titulo, MAX_TITLE)?;
    if title.is_empty() {
        return Err(Erro::Invalido("Escreva o que vai fazer."));
    }
    let size = SIZES
        .into_iter()
        .find(|s| *s == form.tamanho)
        .ok_or(Erro::Invalido("Tamanho deve ser P, M ou G."))?;
    let task = db::NewTask {
        week_id: week.id,
        owner: &user.login,
        project_id: project.id,
        title: &title,
        size,
    };
    db::insert_task(&state.db, &task, ctx.now).await?;
    Ok(changed(&state))
}

/// Tarefa que o usuário pode alterar: dele e de uma semana não encerrada.
async fn own_task(
    state: &AppState,
    ctx: &Ctx,
    user: &CurrentUser,
    id: i64,
) -> Result<db::TaskRow, Erro> {
    let task = db::task(&state.db, id).await?.ok_or(Erro::NaoEncontrado)?;
    if task.owner != user.login {
        return Err(Erro::Proibido("Só quem criou pode mexer nessa tarefa."));
    }
    let week =
        to_week(db::week_by_id(&state.db, task.week_id).await?)?.ok_or(Erro::NaoEncontrado)?;
    if !ctx.editable(&week) {
        return Err(Erro::Proibido("Essa semana já terminou."));
    }
    Ok(task)
}

#[derive(Deserialize)]
pub struct Mover {
    para: String,
}

async fn move_task(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<i64>,
    Form(form): Form<Mover>,
) -> Resultado {
    let ctx = context(&state).await?;
    let task = own_task(&state, &ctx, &user, id).await?;
    let status = column(&form.para).ok_or(Erro::Invalido("Coluna inválida."))?;
    if task.status != status {
        db::set_task_status(&state.db, id, status, ctx.now).await?;
    }
    Ok(changed(&state))
}

async fn delete_task(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> Resultado {
    let ctx = context(&state).await?;
    own_task(&state, &ctx, &user, id).await?;
    db::delete_task(&state.db, id).await?;
    Ok(changed(&state))
}

#[derive(Deserialize)]
pub struct Confirmar {
    semana: String,
}

async fn confirm_week(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Form(form): Form<Confirmar>,
) -> Resultado {
    let ctx = context(&state).await?;
    let week = resolve_week(&state, &ctx, Some(&form.semana)).await?;
    if !ctx.editable(&week) {
        return Err(Erro::Proibido("Essa semana já terminou."));
    }
    let has_tasks = db::tasks_of_week(&state.db, week.id)
        .await?
        .iter()
        .any(|t| t.owner == user.login);
    if !has_tasks {
        return Err(Erro::Invalido("Crie as tarefas antes de confirmar."));
    }
    if !db::confirm_week(&state.db, week.id, &user.login, ctx.now).await? {
        return Err(Erro::Conflito("Essa semana já foi confirmada."));
    }
    Ok(changed(&state))
}

/// Campos `status-<id>` e `nota-<id>`, um par por tarefa.
async fn save_checkin(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Form(form): Form<Vec<(String, String)>>,
) -> Resultado {
    let ctx = context(&state).await?;
    let mine: HashSet<i64> = db::tasks_of_week(&state.db, ctx.current.id)
        .await?
        .into_iter()
        .filter(|t| t.owner == user.login)
        .map(|t| t.id)
        .collect();
    let task_id = |raw: &str| -> Result<i64, Erro> {
        let id = raw
            .parse()
            .map_err(|_| Erro::Invalido("Tarefa inválida."))?;
        if mine.contains(&id) {
            Ok(id)
        } else {
            Err(Erro::Proibido("Essa tarefa não é sua nesta semana."))
        }
    };

    let mut statuses: HashMap<i64, &'static str> = HashMap::new();
    let mut notes: HashMap<i64, String> = HashMap::new();
    for (key, value) in &form {
        if let Some(raw) = key.strip_prefix("status-") {
            let status = CHECKIN
                .iter()
                .map(|(k, _)| *k)
                .find(|k| k == value)
                .ok_or(Erro::Invalido("Resposta inválida no check-in."))?;
            statuses.insert(task_id(raw)?, status);
        } else if let Some(raw) = key.strip_prefix("nota-") {
            notes.insert(task_id(raw)?, one_line(value, MAX_NOTE)?);
        }
    }
    let mut items: Vec<db::CheckinItem> = statuses
        .into_iter()
        .map(|(task_id, status)| db::CheckinItem {
            task_id,
            status,
            note: notes.remove(&task_id).unwrap_or_default(),
        })
        .collect();
    items.sort_by_key(|i| i.task_id);

    let today = ctx.today.to_string();
    db::save_checkin(
        &state.db,
        &user.login,
        &today,
        ctx.current.id,
        &items,
        ctx.now,
    )
    .await?;
    state.ritmo.notify();
    Ok((
        [(
            HeaderName::from_static("hx-redirect"),
            HeaderValue::from_static("/ritmo"),
        )],
        StatusCode::OK,
    )
        .into_response())
}

#[derive(Deserialize)]
pub struct Config {
    dia: String,
}

async fn save_config(State(state): State<AppState>, Form(form): Form<Config>) -> Resultado {
    let day = Weekday::from_key(&form.dia).ok_or(Erro::Invalido("Dia inválido."))?;
    db::set_setting(&state.db, "meeting_day", day.key()).await?;
    Ok(changed_refresh(&state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_cleans_and_limits() {
        assert_eq!(one_line("  a\n b\t\tc  ", 10).unwrap(), "a b c");
        assert_eq!(one_line("x\u{7}y", 10).unwrap(), "xy");
        assert_eq!(one_line("ééé", 3).unwrap(), "ééé");
        assert!(one_line("éééé", 3).is_err());
    }

    #[test]
    fn colors_are_validated() {
        assert_eq!(color(" #A1b2C3 ").as_deref(), Some("#a1b2c3"));
        assert_eq!(color("a1b2c3"), None);
        assert_eq!(color("#a1b2c"), None);
        assert_eq!(color("#a1b2cg"), None);
        assert_eq!(color("red"), None);
    }

    #[test]
    fn columns_and_statuses() {
        assert_eq!(column("doing"), Some("doing"));
        assert_eq!(column("fazendo"), None);
        assert_eq!(status_label("blocked"), ("blocked", "travei"));
        assert_eq!(status_label("???"), ("untouched", "não mexi"));
    }

    #[test]
    fn internal_errors_hide_details() {
        let res = Erro::Interno(sqlx::Error::Protocol("x".into())).into_response();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let res = Erro::Conflito("c").into_response();
        assert_eq!(res.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn bad_week_rows_are_errors() {
        let row = db::WeekRow {
            id: 1,
            start_date: "ontem".into(),
            end_date: "2026-10-20".into(),
        };
        assert!(Week::try_from(row).is_err());
    }
}
