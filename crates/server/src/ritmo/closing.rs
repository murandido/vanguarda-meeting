//! Week closing (KB-07).
//!
//! From the meeting day on, each person closes their own side of the week:
//! the page shows promised × done, extras and blocked tasks, and every
//! unfinished task must be carried over to the next week or dropped. Closing
//! is idempotent, and afterwards that side of the week is read-only. The
//! totals and each task's fate are stored for the monthly retrospective.

use std::collections::{HashMap, HashSet};

use askama::Template;
use axum::{
    Extension, Form,
    extract::{Query, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
};

use super::{
    COLUMNS, Ctx, Erro, Resultado, VerQuery, Week, context, db, ensure_week, people, resolve_week,
};
use crate::{AppState, auth::CurrentUser, pages::render};

/// A week can be closed on its last day (the meeting) or any time after.
pub(super) fn can_close(ctx: &Ctx, week: &Week) -> bool {
    ctx.today >= week.end
}

/// Rounded completion percentage; `None` when nothing was promised.
pub(super) fn percent(done: i64, total: i64) -> Option<i64> {
    (total > 0).then(|| (done * 100 + total / 2) / total)
}

pub(super) async fn closed_by(state: &AppState, week_id: i64, login: &str) -> Result<bool, Erro> {
    Ok(db::closings(&state.db, week_id)
        .await?
        .iter()
        .any(|c| c.login == login))
}

/// The week right before `week`, if `me` had tasks in it and has not closed
/// it yet: the board reminds them.
pub(super) async fn unclosed_before(
    state: &AppState,
    week: &Week,
    me: &str,
) -> Result<Option<String>, Erro> {
    let Some(prev) = db::week_before(&state.db, &week.start.to_string()).await? else {
        return Ok(None);
    };
    let had_tasks = db::tasks_of_week(&state.db, prev.id)
        .await?
        .iter()
        .any(|t| t.owner == me);
    if had_tasks && !closed_by(state, prev.id, me).await? {
        Ok(Some(prev.start_date))
    } else {
        Ok(None)
    }
}

/// The automatic summary of one person's week.
#[derive(Debug)]
struct Summary<'a> {
    promised: i64,
    promised_done: i64,
    extras_done: i64,
    blocked: Vec<&'a db::TaskRow>,
    unfinished: Vec<&'a db::TaskRow>,
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// `tasks` and `promised` must already be filtered to one person.
fn summarize<'a>(
    tasks: &'a [db::TaskRow],
    promised: &[db::PromisedRow],
    blocked: &HashSet<i64>,
) -> Summary<'a> {
    let done: HashSet<i64> = tasks
        .iter()
        .filter(|t| t.status == "done")
        .map(|t| t.id)
        .collect();
    let promised_ids: HashSet<i64> = promised.iter().filter_map(|p| p.task_id).collect();
    Summary {
        promised: count(promised.len()),
        promised_done: count(
            promised
                .iter()
                .filter(|p| p.task_id.is_some_and(|id| done.contains(&id)))
                .count(),
        ),
        extras_done: count(done.difference(&promised_ids).count()),
        blocked: tasks.iter().filter(|t| blocked.contains(&t.id)).collect(),
        unfinished: tasks.iter().filter(|t| t.status != "done").collect(),
    }
}

/// What to do with an unfinished task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    Carry,
    Drop,
}

impl Fate {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "levar" => Some(Self::Carry),
            "descartar" => Some(Self::Drop),
            _ => None,
        }
    }

    fn outcome(self) -> &'static str {
        match self {
            Self::Carry => "carried",
            Self::Drop => "dropped",
        }
    }
}

/// Reads the decisions from the form: `destino-<id>` per task, or `todas`
/// for all of them at once (the bulk buttons). Every unfinished task of
/// `me` needs one; a task of somebody else is forbidden.
fn decide(
    form: &[(String, String)],
    week_tasks: &[db::TaskRow],
    me: &str,
    unfinished: &[&db::TaskRow],
) -> Result<HashMap<i64, Fate>, Erro> {
    let mut fates = HashMap::new();
    let mut all = None;
    for (key, value) in form {
        if key == "todas" {
            all = Some(Fate::parse(value).ok_or(Erro::Invalido("Ação inválida."))?);
        } else if let Some(raw) = key.strip_prefix("destino-") {
            let id: i64 = raw
                .parse()
                .map_err(|_| Erro::Invalido("Tarefa inválida."))?;
            let task = week_tasks
                .iter()
                .find(|t| t.id == id)
                .ok_or(Erro::Invalido("Tarefa inválida."))?;
            if task.owner != me {
                return Err(Erro::Proibido("Essa tarefa não é sua."));
            }
            if task.status == "done" {
                return Err(Erro::Invalido("Essa tarefa já foi feita."));
            }
            let fate = Fate::parse(value).ok_or(Erro::Invalido("Ação inválida."))?;
            fates.insert(id, fate);
        }
    }
    if let Some(fate) = all {
        return Ok(unfinished.iter().map(|t| (t.id, fate)).collect());
    }
    if unfinished.iter().any(|t| !fates.contains_key(&t.id)) {
        return Err(Erro::Invalido(
            "Escolha levar ou descartar cada tarefa não concluída.",
        ));
    }
    Ok(fates)
}

/// Where carried tasks go: the week after the closed one, or the current
/// week when closing late; skipping a week `login` already closed.
async fn carry_target(state: &AppState, ctx: &Ctx, week: &Week, login: &str) -> Result<Week, Erro> {
    let mut day = week.end.add_days(1).max(ctx.today);
    loop {
        let target = ensure_week(&state.db, day, ctx.meeting).await?;
        if !closed_by(state, target.id, login).await? {
            return Ok(target);
        }
        day = target.end.add_days(1);
    }
}

fn redirect(to: &str) -> Resultado {
    let value = HeaderValue::from_str(to).map_err(|_| Erro::Invalido("Semana inválida."))?;
    Ok((
        [(HeaderName::from_static("hx-redirect"), value)],
        StatusCode::OK,
    )
        .into_response())
}

pub(super) async fn close(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Form(form): Form<Vec<(String, String)>>,
) -> Resultado {
    let ctx = context(&state).await?;
    let semana = form
        .iter()
        .find(|(k, _)| k == "semana")
        .map(|(_, v)| v.as_str())
        .ok_or(Erro::Invalido("Semana inválida."))?;
    let week = resolve_week(&state, &ctx, Some(semana)).await?;
    if !can_close(&ctx, &week) {
        return Err(Erro::Proibido(
            "A semana só pode ser fechada a partir do dia da reunião.",
        ));
    }
    let back = format!("/ritmo?semana={}", week.start);
    if closed_by(&state, week.id, &user.login).await? {
        // Idempotent: closing again changes nothing.
        return redirect(&back);
    }

    let week_tasks = db::tasks_of_week(&state.db, week.id).await?;
    let mine: Vec<db::TaskRow> = week_tasks
        .iter()
        .filter(|t| t.owner == user.login)
        .cloned()
        .collect();
    let promised: Vec<db::PromisedRow> = db::promised(&state.db, week.id)
        .await?
        .into_iter()
        .filter(|p| p.login == user.login)
        .collect();
    let blocked: HashSet<i64> = db::blocked_tasks(&state.db, week.id, &user.login)
        .await?
        .into_iter()
        .collect();
    let summary = summarize(&mine, &promised, &blocked);
    let fates = decide(&form, &week_tasks, &user.login, &summary.unfinished)?;

    let promised_ids: HashSet<i64> = promised.iter().filter_map(|p| p.task_id).collect();
    let tasks: Vec<db::ClosingTask> = mine
        .iter()
        .map(|t| db::ClosingTask {
            task: t,
            promised: promised_ids.contains(&t.id),
            was_blocked: blocked.contains(&t.id),
            outcome: fates.get(&t.id).map_or("done", |f| f.outcome()),
        })
        .collect();
    let totals = db::ClosingTotals {
        promised: summary.promised,
        promised_done: summary.promised_done,
        extras_done: summary.extras_done,
        blocked: count(summary.blocked.len()),
    };
    let target = carry_target(&state, &ctx, &week, &user.login).await?;
    db::close_week(
        &state.db,
        week.id,
        &user.login,
        target.id,
        &totals,
        &tasks,
        ctx.now,
    )
    .await?;
    state.ritmo.notify();
    if fates.values().any(|f| *f == Fate::Carry) {
        redirect(&format!("/ritmo?semana={}", target.start))
    } else {
        redirect(&back)
    }
}

// ---------- Page ----------

struct TaskLine {
    id: i64,
    title: String,
    project: String,
    color: String,
    column: &'static str,
}

struct OutcomeLine {
    title: String,
    color: String,
    key: String,
    label: &'static str,
    promised: bool,
    blocked: bool,
}

// Template view: the flags become conditions in the HTML.
#[allow(clippy::struct_excessive_bools)]
struct PersonSummary {
    login: String,
    mine: bool,
    closed: Option<String>,
    promised: i64,
    promised_done: i64,
    percent: Option<i64>,
    extras_done: i64,
    carried: i64,
    dropped: i64,
    blocked: Vec<String>,
    outcomes: Vec<OutcomeLine>,
    unfinished: Vec<TaskLine>,
}

#[derive(Template)]
#[template(path = "ritmo/fechar.html")]
struct ClosePage {
    user: String,
    semana: String,
    label: String,
    open_from: String,
    can_close: bool,
    people: Vec<PersonSummary>,
}

fn outcome_label(key: &str) -> &'static str {
    match key {
        "done" => "feita",
        "carried" => "levada para a próxima semana",
        _ => "descartada",
    }
}

fn column_label(key: &str) -> &'static str {
    COLUMNS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("", |(_, label)| label)
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Query(query): Query<VerQuery>,
) -> Resultado {
    let ctx = context(&state).await?;
    let week = resolve_week(&state, &ctx, query.semana.as_deref()).await?;
    let can_close = can_close(&ctx, &week);
    let tasks = db::tasks_of_week(&state.db, week.id).await?;
    let promised = db::promised(&state.db, week.id).await?;
    let closings = db::closings(&state.db, week.id).await?;
    let closing_tasks = db::closing_tasks(&state.db, week.id).await?;

    let mut summaries = Vec::new();
    for login in people(&state, &user.login) {
        let mine = login == user.login;
        if let Some(c) = closings.iter().find(|c| c.login == login) {
            let rows: Vec<&db::ClosingTaskRow> =
                closing_tasks.iter().filter(|t| t.login == login).collect();
            summaries.push(PersonSummary {
                mine,
                closed: Some(ctx.stamp(c.closed_at)),
                promised: c.promised,
                promised_done: c.promised_done,
                percent: percent(c.promised_done, c.promised),
                extras_done: c.extras_done,
                carried: c.carried,
                dropped: c.dropped,
                blocked: rows
                    .iter()
                    .filter(|t| t.was_blocked)
                    .map(|t| t.title.clone())
                    .collect(),
                outcomes: rows
                    .iter()
                    .map(|t| OutcomeLine {
                        title: t.title.clone(),
                        color: t.project_color.clone(),
                        label: outcome_label(&t.outcome),
                        key: t.outcome.clone(),
                        promised: t.promised,
                        blocked: t.was_blocked,
                    })
                    .collect(),
                unfinished: Vec::new(),
                login,
            });
            continue;
        }
        let theirs: Vec<db::TaskRow> = tasks.iter().filter(|t| t.owner == login).cloned().collect();
        let their_promised: Vec<db::PromisedRow> = promised
            .iter()
            .filter(|p| p.login == login)
            .cloned()
            .collect();
        let blocked: HashSet<i64> = db::blocked_tasks(&state.db, week.id, &login)
            .await?
            .into_iter()
            .collect();
        let s = summarize(&theirs, &their_promised, &blocked);
        summaries.push(PersonSummary {
            mine,
            closed: None,
            promised: s.promised,
            promised_done: s.promised_done,
            percent: percent(s.promised_done, s.promised),
            extras_done: s.extras_done,
            carried: 0,
            dropped: 0,
            blocked: s.blocked.iter().map(|t| t.title.clone()).collect(),
            outcomes: Vec::new(),
            unfinished: s
                .unfinished
                .iter()
                .map(|t| TaskLine {
                    id: t.id,
                    title: t.title.clone(),
                    project: t.project_name.clone(),
                    color: t.project_color.clone(),
                    column: column_label(&t.status),
                })
                .collect(),
            login,
        });
    }

    Ok(render(&ClosePage {
        user: user.login,
        semana: week.start.to_string(),
        label: format!("{} – {}", week.start.short(), week.end.short()),
        open_from: week.end.short(),
        can_close,
        people: summaries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: i64, owner: &str, status: &str) -> db::TaskRow {
        db::TaskRow {
            id,
            week_id: 1,
            owner: owner.into(),
            project_id: 1,
            project_name: "Livro".into(),
            project_color: "#ff8800".into(),
            title: format!("t{id}"),
            size: "M".into(),
            status: status.into(),
            carried_from: None,
        }
    }

    fn promised(ids: &[Option<i64>]) -> Vec<db::PromisedRow> {
        ids.iter()
            .map(|id| db::PromisedRow {
                login: "ana".into(),
                task_id: *id,
            })
            .collect()
    }

    fn form(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn percent_rounds_and_handles_zero() {
        assert_eq!(percent(0, 0), None);
        assert_eq!(percent(2, 3), Some(67));
        assert_eq!(percent(1, 3), Some(33));
        assert_eq!(percent(3, 3), Some(100));
        assert_eq!(percent(1, 8), Some(13));
    }

    #[test]
    fn summary_counts_promised_extras_and_blocked() {
        let tasks = vec![
            task(1, "ana", "done"),
            task(2, "ana", "doing"),
            task(3, "ana", "done"),
            task(4, "ana", "todo"),
        ];
        // 1 and 2 promised, plus one promised task that was deleted.
        let p = promised(&[Some(1), Some(2), None]);
        let blocked: HashSet<i64> = [2].into();
        let s = summarize(&tasks, &p, &blocked);
        assert_eq!((s.promised, s.promised_done, s.extras_done), (3, 1, 1));
        assert_eq!(s.blocked.iter().map(|t| t.id).collect::<Vec<_>>(), [2]);
        assert_eq!(
            s.unfinished.iter().map(|t| t.id).collect::<Vec<_>>(),
            [2, 4]
        );
    }

    #[test]
    fn every_unfinished_task_needs_a_decision() {
        let week = vec![
            task(1, "ana", "todo"),
            task(2, "ana", "doing"),
            task(3, "ana", "done"),
            task(4, "bia", "todo"),
        ];
        let unfinished: Vec<&db::TaskRow> = week[..2].iter().collect();

        let fates = decide(
            &form(&[("destino-1", "levar"), ("destino-2", "descartar")]),
            &week,
            "ana",
            &unfinished,
        )
        .unwrap();
        assert_eq!(fates[&1], Fate::Carry);
        assert_eq!(fates[&2], Fate::Drop);

        // Missing one.
        assert!(matches!(
            decide(&form(&[("destino-1", "levar")]), &week, "ana", &unfinished),
            Err(Erro::Invalido(_))
        ));
        // Bulk action wins over individual choices.
        let fates = decide(
            &form(&[("destino-1", "levar"), ("todas", "descartar")]),
            &week,
            "ana",
            &unfinished,
        )
        .unwrap();
        assert!(fates.values().all(|f| *f == Fate::Drop));
        assert_eq!(fates.len(), 2);
    }

    #[test]
    fn decisions_are_validated() {
        let week = vec![
            task(1, "ana", "todo"),
            task(3, "ana", "done"),
            task(4, "bia", "todo"),
        ];
        let unfinished: Vec<&db::TaskRow> = week[..1].iter().collect();
        let cases: [(&[(&str, &str)], bool); 5] = [
            (&[("destino-4", "levar")], true),
            (&[("destino-3", "levar")], false),
            (&[("destino-99", "levar")], false),
            (&[("destino-x", "levar")], false),
            (&[("destino-1", "talvez")], false),
        ];
        for (pairs, forbidden) in cases {
            let res = decide(&form(pairs), &week, "ana", &unfinished);
            if forbidden {
                assert!(matches!(res, Err(Erro::Proibido(_))), "{pairs:?}");
            } else {
                assert!(matches!(res, Err(Erro::Invalido(_))), "{pairs:?}");
            }
        }
        assert!(decide(&form(&[("todas", "sei-la")]), &week, "ana", &unfinished).is_err());
        // Nothing unfinished: nothing to decide.
        assert!(decide(&[], &week, "ana", &[]).unwrap().is_empty());
    }

    #[test]
    fn labels() {
        assert_eq!(outcome_label("done"), "feita");
        assert_eq!(outcome_label("carried"), "levada para a próxima semana");
        assert_eq!(outcome_label("dropped"), "descartada");
        assert_eq!(column_label("doing"), "Fazendo");
        assert_eq!(column_label("???"), "");
    }
}
