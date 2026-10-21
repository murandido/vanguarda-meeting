//! Streaks on the board and on the home card (KB-06), and the pause routes.
//!
//! The numbers come from [`super::streak::compute`] over the stored check-ins
//! and pauses. The freeze events it derives are recorded as history the
//! first time they are seen.

use std::collections::{BTreeMap, HashSet};

use axum::{
    Extension, Form,
    extract::{Path, State},
};
use serde::Deserialize;

use super::{
    Ctx, Erro, Resultado, changed_refresh, context,
    date::Date,
    db, one_line,
    streak::{self, History, Pause, PauseError},
};
use crate::{AppState, auth::CurrentUser};

const MAX_REASON: usize = 60;

/// One person's counter on the board.
pub(super) struct StreakView {
    pub login: String,
    pub mine: bool,
    pub current: u32,
    pub days: &'static str,
    pub freezes: u8,
    /// `feito`, `pausa` or `pendente`: drives the hint and its style.
    pub status: &'static str,
    pub hint: String,
}

pub(super) struct Streaks {
    pub people: Vec<StreakView>,
    pub pair: u32,
    pub pair_days: &'static str,
    pub pair_today: bool,
    pub max_freezes: u8,
}

fn days_word(n: u32) -> &'static str {
    if n == 1 { "dia" } else { "dias" }
}

fn parse_date(s: &str) -> sqlx::Result<Date> {
    Date::parse(s).ok_or_else(|| sqlx::Error::Protocol(format!("data inválida: {s}")))
}

fn to_pause(row: &db::PauseRow) -> sqlx::Result<Pause> {
    Ok(Pause {
        start: parse_date(&row.start_date)?,
        end: parse_date(&row.end_date)?,
    })
}

/// Computes everyone's streak (in `logins` order) and records new freeze
/// events. Also returns the pauses, already parsed.
async fn compute(
    state: &AppState,
    ctx: &Ctx,
    logins: &[String],
) -> Result<
    (
        Vec<streak::Streak>,
        streak::PairStreak,
        Vec<(db::PauseRow, Pause)>,
    ),
    Erro,
> {
    let mut histories: BTreeMap<&str, History> = logins
        .iter()
        .map(|l| (l.as_str(), History::default()))
        .collect();
    for row in db::checkin_days(&state.db).await? {
        if let Some(h) = histories.get_mut(row.login.as_str()) {
            h.checkins.insert(parse_date(&row.day)?);
        }
    }
    let mut pauses = Vec::new();
    for row in db::pauses(&state.db).await? {
        let pause = to_pause(&row)?;
        if let Some(h) = histories.get_mut(row.login.as_str()) {
            h.pauses.push(pause);
        }
        pauses.push((row, pause));
    }
    let ordered: Vec<History> = logins
        .iter()
        .map(|l| histories.remove(l.as_str()).unwrap_or_default())
        .collect();
    let (streaks, pair) = streak::compute(&ordered, ctx.today);

    let known: HashSet<db::StreakEventRow> =
        db::streak_events(&state.db).await?.into_iter().collect();
    let new: Vec<db::StreakEventRow> = logins
        .iter()
        .zip(&streaks)
        .flat_map(|(login, s)| {
            s.events.iter().map(|e| db::StreakEventRow {
                login: login.clone(),
                day: e.day.to_string(),
                kind: e.kind.key().to_owned(),
            })
        })
        .filter(|e| !known.contains(e))
        .collect();
    if !new.is_empty() {
        db::insert_streak_events(&state.db, &new, ctx.now).await?;
    }
    Ok((streaks, pair, pauses))
}

pub(super) async fn load(
    state: &AppState,
    ctx: &Ctx,
    me: &str,
    logins: &[String],
) -> Result<Streaks, Erro> {
    let (streaks, pair, pauses) = compute(state, ctx, logins).await?;
    let people = logins
        .iter()
        .zip(streaks)
        .map(|(login, s)| {
            let (status, hint) = if s.paused_today {
                let until = pauses
                    .iter()
                    .find(|(row, p)| &row.login == login && p.contains(ctx.today))
                    .map(|(_, p)| p.end.short())
                    .unwrap_or_default();
                ("pausa", format!("em pausa até {until}"))
            } else if s.checked_in_today {
                ("feito", "check-in de hoje feito".to_owned())
            } else if s.current > 0 {
                ("pendente", "falta o check-in de hoje".to_owned())
            } else {
                ("pendente", "comece hoje com um check-in".to_owned())
            };
            StreakView {
                login: login.clone(),
                mine: login == me,
                current: s.current,
                days: days_word(s.current),
                freezes: s.freezes,
                status,
                hint,
            }
        })
        .collect();
    Ok(Streaks {
        people,
        pair: pair.current,
        pair_days: days_word(pair.current),
        pair_today: pair.everyone_today,
        max_freezes: streak::MAX_FREEZES,
    })
}

/// Streak counters for the Ritmo card on the home page.
pub struct HomeCard {
    pub people: Vec<HomeStreak>,
    pub pair: u32,
}

pub struct HomeStreak {
    pub label: String,
    pub current: u32,
}

/// The home card's numbers; `None` (and a log line) if anything fails, so the
/// home page never breaks because of the Ritmo.
pub async fn home_card(state: &AppState, me: &str) -> Option<HomeCard> {
    let result = async {
        let ctx = context(state).await?;
        let logins = super::people(state, me);
        let (streaks, pair, _) = compute(state, &ctx, &logins).await?;
        Ok::<_, Erro>(HomeCard {
            people: logins
                .iter()
                .zip(streaks)
                .map(|(login, s)| HomeStreak {
                    label: if login == me {
                        "você".to_owned()
                    } else {
                        login.clone()
                    },
                    current: s.current,
                })
                .collect(),
            pair: pair.current,
        })
    }
    .await;
    match result {
        Ok(card) => Some(card),
        Err(e) => {
            tracing::error!("streaks on the home page: {e:?}");
            None
        }
    }
}

// ---------- Pauses ----------

pub(super) struct PauseView {
    pub id: i64,
    pub login: String,
    pub mine: bool,
    pub label: String,
    pub reason: String,
    pub can_cancel: bool,
}

/// Pauses that have not ended yet, of everyone.
pub(super) async fn pause_views(
    state: &AppState,
    ctx: &Ctx,
    me: &str,
) -> Result<Vec<PauseView>, Erro> {
    let mut views = Vec::new();
    for row in db::pauses(&state.db).await? {
        let p = to_pause(&row)?;
        if p.end < ctx.today {
            continue;
        }
        views.push(PauseView {
            id: row.id,
            mine: row.login == me,
            can_cancel: row.login == me && p.start > ctx.today,
            label: if p.start == p.end {
                p.start.short()
            } else {
                format!("{} – {}", p.start.short(), p.end.short())
            },
            login: row.login,
            reason: row.reason,
        });
    }
    Ok(views)
}

#[derive(Deserialize)]
pub struct NovaPausa {
    inicio: String,
    fim: String,
    #[serde(default)]
    motivo: String,
}

pub(super) async fn create_pause(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Form(form): Form<NovaPausa>,
) -> Resultado {
    let ctx = context(&state).await?;
    let start = Date::parse(&form.inicio).ok_or(Erro::Invalido("Data de início inválida."))?;
    let end = Date::parse(&form.fim).ok_or(Erro::Invalido("Data de fim inválida."))?;
    let reason = one_line(&form.motivo, MAX_REASON)?;
    let mut mine = Vec::new();
    for row in db::pauses(&state.db).await? {
        if row.login == user.login {
            mine.push(to_pause(&row)?);
        }
    }
    streak::validate_pause(Pause { start, end }, ctx.today, &mine).map_err(|e| {
        Erro::Invalido(match e {
            PauseError::NotAhead => "A pausa é combinada antes: comece a partir de amanhã.",
            PauseError::Backwards => "O fim da pausa vem antes do início.",
            PauseError::TooLong => "A pausa pode ter no máximo 30 dias.",
            PauseError::Overlaps => "Você já tem uma pausa nesses dias.",
        })
    })?;
    let pause = db::NewPause {
        login: &user.login,
        start: &start.to_string(),
        end: &end.to_string(),
        reason: &reason,
    };
    db::insert_pause(&state.db, &pause, ctx.now).await?;
    Ok(changed_refresh(&state))
}

/// Only the owner can cancel, and only before the pause starts: past days
/// must keep their meaning.
pub(super) async fn cancel_pause(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> Resultado {
    let ctx = context(&state).await?;
    let row = db::pause(&state.db, id).await?.ok_or(Erro::NaoEncontrado)?;
    if row.login != user.login {
        return Err(Erro::Proibido("Essa pausa não é sua."));
    }
    if to_pause(&row)?.start <= ctx.today {
        return Err(Erro::Proibido("Essa pausa já começou."));
    }
    db::delete_pause(&state.db, id).await?;
    Ok(changed_refresh(&state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pluralizes_days() {
        assert_eq!(days_word(0), "dias");
        assert_eq!(days_word(1), "dia");
        assert_eq!(days_word(2), "dias");
    }

    #[test]
    fn bad_pause_rows_are_errors() {
        let row = db::PauseRow {
            id: 1,
            login: "ana".into(),
            start_date: "amanhã".into(),
            end_date: "2026-10-20".into(),
            reason: String::new(),
        };
        assert!(to_pause(&row).is_err());
    }
}
