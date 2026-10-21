//! Queries do Ritmo. Datas civis são texto `YYYY-MM-DD`; instantes, segundos
//! Unix vindos do [`crate::Clock`].

use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub struct WeekRow {
    pub id: i64,
    pub start_date: String,
    pub end_date: String,
}

#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: i64,
    pub week_id: i64,
    pub owner: String,
    pub project_id: i64,
    pub project_name: String,
    pub project_color: String,
    pub title: String,
    pub size: String,
    pub status: String,
    pub carried_from: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CommitmentRow {
    pub login: String,
    pub confirmed_at: i64,
}

#[derive(Debug, Clone)]
pub struct PromisedRow {
    pub login: String,
    pub task_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CheckinItemRow {
    pub task_id: i64,
    pub title: String,
    pub project_color: String,
    pub status: String,
    pub note: String,
}

pub async fn setting(db: &SqlitePool, key: &str) -> sqlx::Result<Option<String>> {
    let row = sqlx::query!("SELECT value FROM ritmo_settings WHERE key = ?", key)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.value))
}

pub async fn set_setting(db: &SqlitePool, key: &str, value: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO ritmo_settings (key, value) VALUES (?, ?)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        key,
        value
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn week_containing(db: &SqlitePool, day: &str) -> sqlx::Result<Option<WeekRow>> {
    sqlx::query_as!(
        WeekRow,
        "SELECT id, start_date, end_date FROM ritmo_weeks
         WHERE start_date <= ?1 AND end_date >= ?1",
        day
    )
    .fetch_optional(db)
    .await
}

pub async fn week_by_start(db: &SqlitePool, start: &str) -> sqlx::Result<Option<WeekRow>> {
    sqlx::query_as!(
        WeekRow,
        "SELECT id, start_date, end_date FROM ritmo_weeks WHERE start_date = ?",
        start
    )
    .fetch_optional(db)
    .await
}

pub async fn week_by_id(db: &SqlitePool, id: i64) -> sqlx::Result<Option<WeekRow>> {
    sqlx::query_as!(
        WeekRow,
        "SELECT id, start_date, end_date FROM ritmo_weeks WHERE id = ?",
        id
    )
    .fetch_optional(db)
    .await
}

/// Última semana que termina antes de `day`.
pub async fn week_before(db: &SqlitePool, day: &str) -> sqlx::Result<Option<WeekRow>> {
    sqlx::query_as!(
        WeekRow,
        "SELECT id, start_date, end_date FROM ritmo_weeks
         WHERE end_date < ? ORDER BY end_date DESC LIMIT 1",
        day
    )
    .fetch_optional(db)
    .await
}

/// Primeira semana que começa depois de `day`.
pub async fn week_after(db: &SqlitePool, day: &str) -> sqlx::Result<Option<WeekRow>> {
    sqlx::query_as!(
        WeekRow,
        "SELECT id, start_date, end_date FROM ritmo_weeks
         WHERE start_date > ? ORDER BY start_date LIMIT 1",
        day
    )
    .fetch_optional(db)
    .await
}

/// Cria a semana se ainda não existir uma com o mesmo início.
pub async fn insert_week(db: &SqlitePool, start: &str, end: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO ritmo_weeks (start_date, end_date) VALUES (?, ?)
         ON CONFLICT (start_date) DO NOTHING",
        start,
        end
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn projects(db: &SqlitePool) -> sqlx::Result<Vec<ProjectRow>> {
    sqlx::query_as!(
        ProjectRow,
        "SELECT id, owner, name, color FROM ritmo_projects ORDER BY owner, name COLLATE NOCASE"
    )
    .fetch_all(db)
    .await
}

pub async fn project(db: &SqlitePool, id: i64) -> sqlx::Result<Option<ProjectRow>> {
    sqlx::query_as!(
        ProjectRow,
        "SELECT id, owner, name, color FROM ritmo_projects WHERE id = ?",
        id
    )
    .fetch_optional(db)
    .await
}

/// Cria o projeto; `Ok(false)` se o dono já tiver um com esse nome.
pub async fn insert_project(
    db: &SqlitePool,
    owner: &str,
    name: &str,
    color: &str,
    now: i64,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "INSERT INTO ritmo_projects (owner, name, color, created_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (owner, name) DO NOTHING",
        owner,
        name,
        color,
        now
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn tasks_of_week(db: &SqlitePool, week_id: i64) -> sqlx::Result<Vec<TaskRow>> {
    sqlx::query_as!(
        TaskRow,
        "SELECT t.id, t.week_id, t.owner, t.project_id, p.name AS project_name,
                p.color AS project_color, t.title, t.size, t.status, t.carried_from
         FROM ritmo_tasks t JOIN ritmo_projects p ON p.id = t.project_id
         WHERE t.week_id = ? ORDER BY t.id",
        week_id
    )
    .fetch_all(db)
    .await
}

pub async fn task(db: &SqlitePool, id: i64) -> sqlx::Result<Option<TaskRow>> {
    sqlx::query_as!(
        TaskRow,
        "SELECT t.id, t.week_id, t.owner, t.project_id, p.name AS project_name,
                p.color AS project_color, t.title, t.size, t.status, t.carried_from
         FROM ritmo_tasks t JOIN ritmo_projects p ON p.id = t.project_id
         WHERE t.id = ?",
        id
    )
    .fetch_optional(db)
    .await
}

pub struct NewTask<'a> {
    pub week_id: i64,
    pub owner: &'a str,
    pub project_id: i64,
    pub title: &'a str,
    pub size: &'a str,
}

pub async fn insert_task(db: &SqlitePool, task: &NewTask<'_>, now: i64) -> sqlx::Result<i64> {
    let result = sqlx::query!(
        "INSERT INTO ritmo_tasks (week_id, owner, project_id, title, size, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        task.week_id,
        task.owner,
        task.project_id,
        task.title,
        task.size,
        now,
        now
    )
    .execute(db)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Muda a coluna da tarefa; `done_at` acompanha a entrada (e saída) de Feito.
pub async fn set_task_status(db: &SqlitePool, id: i64, status: &str, now: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE ritmo_tasks
         SET status = ?2, updated_at = ?3,
             done_at = CASE WHEN ?2 = 'done' THEN COALESCE(done_at, ?3) ELSE NULL END
         WHERE id = ?1",
        id,
        status,
        now
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_task(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM ritmo_tasks WHERE id = ?", id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn commitments(db: &SqlitePool, week_id: i64) -> sqlx::Result<Vec<CommitmentRow>> {
    sqlx::query_as!(
        CommitmentRow,
        "SELECT login, confirmed_at FROM ritmo_commitments WHERE week_id = ?",
        week_id
    )
    .fetch_all(db)
    .await
}

pub async fn promised(db: &SqlitePool, week_id: i64) -> sqlx::Result<Vec<PromisedRow>> {
    sqlx::query_as!(
        PromisedRow,
        "SELECT login, task_id FROM ritmo_commitment_tasks WHERE week_id = ? ORDER BY id",
        week_id
    )
    .fetch_all(db)
    .await
}

/// Congela as tarefas atuais de `login` na semana como "prometido".
/// `Ok(false)` se a semana já tinha sido confirmada.
pub async fn confirm_week(
    db: &SqlitePool,
    week_id: i64,
    login: &str,
    now: i64,
) -> sqlx::Result<bool> {
    let mut tx = db.begin().await?;
    let inserted = sqlx::query!(
        "INSERT INTO ritmo_commitments (week_id, login, confirmed_at) VALUES (?, ?, ?)
         ON CONFLICT (week_id, login) DO NOTHING",
        week_id,
        login,
        now
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(false);
    }
    sqlx::query!(
        "INSERT INTO ritmo_commitment_tasks (week_id, login, task_id, project_id, title, size)
         SELECT week_id, owner, id, project_id, title, size FROM ritmo_tasks
         WHERE week_id = ?1 AND owner = ?2 ORDER BY id",
        week_id,
        login
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Itens do check-in de `login` em `day` (vazio se não fez).
pub async fn checkin_items(
    db: &SqlitePool,
    login: &str,
    day: &str,
) -> sqlx::Result<Vec<CheckinItemRow>> {
    sqlx::query_as!(
        CheckinItemRow,
        "SELECT i.task_id, t.title, p.color AS project_color,
                i.status, i.note
         FROM ritmo_checkins c
         JOIN ritmo_checkin_items i ON i.checkin_id = c.id
         JOIN ritmo_tasks t ON t.id = i.task_id
         JOIN ritmo_projects p ON p.id = t.project_id
         WHERE c.login = ? AND c.day = ? ORDER BY t.id",
        login,
        day
    )
    .fetch_all(db)
    .await
}

/// Quem fez check-in em `day` e quando (atualização mais recente).
pub async fn checkins_of_day(db: &SqlitePool, day: &str) -> sqlx::Result<Vec<CommitmentRow>> {
    sqlx::query_as!(
        CommitmentRow,
        r#"SELECT login, updated_at AS "confirmed_at!" FROM ritmo_checkins WHERE day = ?"#,
        day
    )
    .fetch_all(db)
    .await
}

pub struct CheckinItem {
    pub task_id: i64,
    pub status: &'static str,
    pub note: String,
}

/// Cria ou substitui o check-in de `login` em `day`.
pub async fn save_checkin(
    db: &SqlitePool,
    login: &str,
    day: &str,
    week_id: i64,
    items: &[CheckinItem],
    now: i64,
) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    let id = sqlx::query_scalar!(
        r#"INSERT INTO ritmo_checkins (login, day, week_id, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?4)
           ON CONFLICT (login, day) DO UPDATE SET updated_at = excluded.updated_at
           RETURNING id AS "id!""#,
        login,
        day,
        week_id,
        now
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM ritmo_checkin_items WHERE checkin_id = ?", id)
        .execute(&mut *tx)
        .await?;
    for item in items {
        sqlx::query!(
            "INSERT INTO ritmo_checkin_items (checkin_id, task_id, status, note)
             VALUES (?, ?, ?, ?)",
            id,
            item.task_id,
            item.status,
            item.note
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ---------- Streaks (KB-06) ----------

#[derive(Debug, Clone)]
pub struct CheckinDayRow {
    pub login: String,
    pub day: String,
}

#[derive(Debug, Clone)]
pub struct PauseRow {
    pub id: i64,
    pub login: String,
    pub start_date: String,
    pub end_date: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StreakEventRow {
    pub login: String,
    pub day: String,
    pub kind: String,
}

/// Every check-in day of everyone (one row per person per day).
pub async fn checkin_days(db: &SqlitePool) -> sqlx::Result<Vec<CheckinDayRow>> {
    sqlx::query_as!(
        CheckinDayRow,
        "SELECT login, day FROM ritmo_checkins ORDER BY day"
    )
    .fetch_all(db)
    .await
}

pub async fn pauses(db: &SqlitePool) -> sqlx::Result<Vec<PauseRow>> {
    sqlx::query_as!(
        PauseRow,
        "SELECT id, login, start_date, end_date, reason FROM ritmo_pauses
         ORDER BY start_date, login"
    )
    .fetch_all(db)
    .await
}

pub async fn pause(db: &SqlitePool, id: i64) -> sqlx::Result<Option<PauseRow>> {
    sqlx::query_as!(
        PauseRow,
        "SELECT id, login, start_date, end_date, reason FROM ritmo_pauses WHERE id = ?",
        id
    )
    .fetch_optional(db)
    .await
}

pub struct NewPause<'a> {
    pub login: &'a str,
    pub start: &'a str,
    pub end: &'a str,
    pub reason: &'a str,
}

pub async fn insert_pause(db: &SqlitePool, pause: &NewPause<'_>, now: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO ritmo_pauses (login, start_date, end_date, reason, created_at)
         VALUES (?, ?, ?, ?, ?)",
        pause.login,
        pause.start,
        pause.end,
        pause.reason,
        now
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_pause(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM ritmo_pauses WHERE id = ?", id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn streak_events(db: &SqlitePool) -> sqlx::Result<Vec<StreakEventRow>> {
    sqlx::query_as!(
        StreakEventRow,
        "SELECT login, day, kind FROM ritmo_streak_events ORDER BY day"
    )
    .fetch_all(db)
    .await
}

/// Records freeze events; the ones already recorded are left untouched.
pub async fn insert_streak_events(
    db: &SqlitePool,
    events: &[StreakEventRow],
    now: i64,
) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    for e in events {
        sqlx::query!(
            "INSERT INTO ritmo_streak_events (login, day, kind, created_at) VALUES (?, ?, ?, ?)
             ON CONFLICT (login, day, kind) DO NOTHING",
            e.login,
            e.day,
            e.kind,
            now
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

// ---------- Week closing (KB-07) ----------

#[derive(Debug, Clone)]
pub struct ClosingRow {
    pub login: String,
    pub closed_at: i64,
    pub promised: i64,
    pub promised_done: i64,
    pub extras_done: i64,
    pub carried: i64,
    pub dropped: i64,
}

#[derive(Debug, Clone)]
pub struct ClosingTaskRow {
    pub login: String,
    pub title: String,
    pub project_color: String,
    pub promised: bool,
    pub was_blocked: bool,
    pub outcome: String,
}

pub async fn closings(db: &SqlitePool, week_id: i64) -> sqlx::Result<Vec<ClosingRow>> {
    sqlx::query_as!(
        ClosingRow,
        "SELECT login, closed_at, promised, promised_done, extras_done, carried, dropped
         FROM ritmo_week_closings WHERE week_id = ?",
        week_id
    )
    .fetch_all(db)
    .await
}

pub async fn closing_tasks(db: &SqlitePool, week_id: i64) -> sqlx::Result<Vec<ClosingTaskRow>> {
    sqlx::query_as!(
        ClosingTaskRow,
        r#"SELECT c.login, c.title, p.color AS project_color,
                  c.promised AS "promised: bool", c.was_blocked AS "was_blocked: bool",
                  c.outcome
           FROM ritmo_week_closing_tasks c JOIN ritmo_projects p ON p.id = c.project_id
           WHERE c.week_id = ? ORDER BY c.id"#,
        week_id
    )
    .fetch_all(db)
    .await
}

/// Tasks of `login` in the week that were marked "travei" in some check-in.
pub async fn blocked_tasks(db: &SqlitePool, week_id: i64, login: &str) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        "SELECT DISTINCT i.task_id
         FROM ritmo_checkin_items i JOIN ritmo_tasks t ON t.id = i.task_id
         WHERE t.week_id = ? AND t.owner = ? AND i.status = 'blocked'
         ORDER BY i.task_id",
        week_id,
        login
    )
    .fetch_all(db)
    .await
}

/// Totals stored with the closing.
pub struct ClosingTotals {
    pub promised: i64,
    pub promised_done: i64,
    pub extras_done: i64,
    pub blocked: i64,
}

/// One task of the closed week and what happens to it.
pub struct ClosingTask<'a> {
    pub task: &'a TaskRow,
    pub promised: bool,
    pub was_blocked: bool,
    /// `done`, `carried` or `dropped`.
    pub outcome: &'static str,
}

/// Closes the week for `login`, carrying tasks into `next_week_id`.
/// `Ok(false)` if it was already closed: nothing changes (idempotent).
pub async fn close_week(
    db: &SqlitePool,
    week_id: i64,
    login: &str,
    next_week_id: i64,
    totals: &ClosingTotals,
    tasks: &[ClosingTask<'_>],
    now: i64,
) -> sqlx::Result<bool> {
    let count = |outcome: &str| {
        i64::try_from(tasks.iter().filter(|t| t.outcome == outcome).count()).unwrap_or(i64::MAX)
    };
    let (carried, dropped) = (count("carried"), count("dropped"));
    let mut tx = db.begin().await?;
    let inserted = sqlx::query!(
        "INSERT INTO ritmo_week_closings
           (week_id, login, closed_at, promised, promised_done, extras_done, blocked, carried,
            dropped)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (week_id, login) DO NOTHING",
        week_id,
        login,
        now,
        totals.promised,
        totals.promised_done,
        totals.extras_done,
        totals.blocked,
        carried,
        dropped
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(false);
    }
    for t in tasks {
        let carried_task_id = if t.outcome == "carried" {
            let id = sqlx::query!(
                "INSERT INTO ritmo_tasks
                   (week_id, owner, project_id, title, size, status, created_at, updated_at,
                    carried_from)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                next_week_id,
                login,
                t.task.project_id,
                t.task.title,
                t.task.size,
                t.task.status,
                now,
                now,
                t.task.id
            )
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();
            Some(id)
        } else {
            None
        };
        sqlx::query!(
            "INSERT INTO ritmo_week_closing_tasks
               (week_id, login, task_id, project_id, title, size, promised, was_blocked, outcome,
                carried_task_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            week_id,
            login,
            t.task.id,
            t.task.project_id,
            t.task.title,
            t.task.size,
            t.promised,
            t.was_blocked,
            t.outcome,
            carried_task_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}
