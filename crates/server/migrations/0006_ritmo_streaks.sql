-- Ritmo: streaks (KB-06) and week closing (KB-07).
--
-- Streaks themselves are never stored: they are computed from check-ins and
-- pauses. Only the events that make up the history are persisted.

-- Planned breaks (travel, sickness). A paused day neither breaks nor extends
-- the streak. Pauses are scheduled ahead of time and can only be cancelled
-- before they start, so past days never change meaning.
CREATE TABLE ritmo_pauses (
    id         INTEGER PRIMARY KEY NOT NULL,
    login      TEXT NOT NULL REFERENCES users (login) ON DELETE CASCADE ON UPDATE CASCADE,
    start_date TEXT NOT NULL,
    end_date   TEXT NOT NULL,
    reason     TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    CHECK (end_date >= start_date)
);

CREATE INDEX ritmo_pauses_login ON ritmo_pauses (login, start_date);

-- Freezes earned and used, as derived from check-ins and pauses. Written
-- idempotently whenever streaks are computed; kept as history.
CREATE TABLE ritmo_streak_events (
    login      TEXT NOT NULL REFERENCES users (login) ON DELETE CASCADE ON UPDATE CASCADE,
    day        TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('freeze_earned', 'freeze_used')),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (login, day, kind)
);

-- A task carried over from a closed week points to the task it continues.
ALTER TABLE ritmo_tasks
    ADD COLUMN carried_from INTEGER REFERENCES ritmo_tasks (id) ON DELETE SET NULL;

-- Week closing, per person. Once closed, that person's side of the week is
-- read-only. The totals are frozen here for the monthly retrospective.
CREATE TABLE ritmo_week_closings (
    week_id       INTEGER NOT NULL REFERENCES ritmo_weeks (id),
    login         TEXT NOT NULL REFERENCES users (login) ON DELETE CASCADE ON UPDATE CASCADE,
    closed_at     INTEGER NOT NULL,
    promised      INTEGER NOT NULL,
    promised_done INTEGER NOT NULL,
    extras_done   INTEGER NOT NULL,
    blocked       INTEGER NOT NULL,
    carried       INTEGER NOT NULL,
    dropped       INTEGER NOT NULL,
    PRIMARY KEY (week_id, login)
);

-- One row per task of the closed week, with what happened to it. Survives the
-- task being deleted (`task_id` becomes NULL).
CREATE TABLE ritmo_week_closing_tasks (
    id              INTEGER PRIMARY KEY NOT NULL,
    week_id         INTEGER NOT NULL,
    login           TEXT NOT NULL,
    task_id         INTEGER REFERENCES ritmo_tasks (id) ON DELETE SET NULL,
    project_id      INTEGER NOT NULL REFERENCES ritmo_projects (id),
    title           TEXT NOT NULL,
    size            TEXT NOT NULL,
    promised        INTEGER NOT NULL CHECK (promised IN (0, 1)),
    was_blocked     INTEGER NOT NULL CHECK (was_blocked IN (0, 1)),
    outcome         TEXT NOT NULL CHECK (outcome IN ('done', 'carried', 'dropped')),
    carried_task_id INTEGER REFERENCES ritmo_tasks (id) ON DELETE SET NULL,
    FOREIGN KEY (week_id, login) REFERENCES ritmo_week_closings (week_id, login)
        ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE INDEX ritmo_week_closing_tasks_week ON ritmo_week_closing_tasks (week_id, login);
