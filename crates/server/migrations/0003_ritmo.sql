-- Ritmo: Kanban semanal com compromisso ("prometido") e check-in diário.
-- Datas civis em horário local (`YYYY-MM-DD`); instantes em segundos Unix,
-- vindos do relógio do servidor (injetável nos testes).

-- Configurações compartilhadas da dupla.
--   meeting_day:        dia da reunião (em inglês: monday .. sunday). A semana
--                       termina no fim desse dia e a próxima começa em seguida.
--   utc_offset_minutes: fuso usado para saber qual é "hoje" (-180 = Brasília).
CREATE TABLE ritmo_settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

INSERT INTO ritmo_settings (key, value) VALUES
    ('meeting_day', 'tuesday'),
    ('utc_offset_minutes', '-180');

-- Semanas, de `start_date` até `end_date` (o dia da reunião), inclusive.
-- São criadas sob demanda e nunca se sobrepõem: mudar o dia da reunião só
-- afeta as semanas que ainda não existem.
CREATE TABLE ritmo_weeks (
    id         INTEGER PRIMARY KEY NOT NULL,
    start_date TEXT NOT NULL UNIQUE,
    end_date   TEXT NOT NULL,
    CHECK (end_date >= start_date)
);

-- Projetos pessoais: cada um tem os seus.
CREATE TABLE ritmo_projects (
    id         INTEGER PRIMARY KEY NOT NULL,
    owner      TEXT NOT NULL REFERENCES users (login) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    color      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (owner, name)
);

-- Tarefas da semana. `done_at` guarda quando foi para Feito (para placar e
-- histórico no futuro).
CREATE TABLE ritmo_tasks (
    id         INTEGER PRIMARY KEY NOT NULL,
    week_id    INTEGER NOT NULL REFERENCES ritmo_weeks (id),
    owner      TEXT NOT NULL REFERENCES users (login) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES ritmo_projects (id),
    title      TEXT NOT NULL,
    size       TEXT NOT NULL CHECK (size IN ('P', 'M', 'G')),
    status     TEXT NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'doing', 'done')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    done_at    INTEGER
);

CREATE INDEX ritmo_tasks_week_owner ON ritmo_tasks (week_id, owner);

-- Compromisso semanal: ao confirmar, a lista do momento vira o "prometido".
CREATE TABLE ritmo_commitments (
    week_id      INTEGER NOT NULL REFERENCES ritmo_weeks (id),
    login        TEXT NOT NULL REFERENCES users (login) ON DELETE CASCADE,
    confirmed_at INTEGER NOT NULL,
    PRIMARY KEY (week_id, login)
);

-- Cópia congelada de cada tarefa prometida. Sobrevive à tarefa ser apagada
-- (`task_id` vira NULL), para a comparação prometido × feito.
CREATE TABLE ritmo_commitment_tasks (
    id         INTEGER PRIMARY KEY NOT NULL,
    week_id    INTEGER NOT NULL,
    login      TEXT NOT NULL,
    task_id    INTEGER REFERENCES ritmo_tasks (id) ON DELETE SET NULL,
    project_id INTEGER NOT NULL REFERENCES ritmo_projects (id),
    title      TEXT NOT NULL,
    size       TEXT NOT NULL,
    FOREIGN KEY (week_id, login) REFERENCES ritmo_commitments (week_id, login) ON DELETE CASCADE
);

CREATE INDEX ritmo_commitment_tasks_week ON ritmo_commitment_tasks (week_id, login);

-- Check-in diário: um por pessoa por dia (editável no mesmo dia).
CREATE TABLE ritmo_checkins (
    id         INTEGER PRIMARY KEY NOT NULL,
    login      TEXT NOT NULL REFERENCES users (login) ON DELETE CASCADE,
    day        TEXT NOT NULL,
    week_id    INTEGER NOT NULL REFERENCES ritmo_weeks (id),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (login, day)
);

CREATE INDEX ritmo_checkins_day ON ritmo_checkins (day);

-- O que cada tarefa teve no check-in: avancei / travei / não mexi + 1 linha.
CREATE TABLE ritmo_checkin_items (
    checkin_id INTEGER NOT NULL REFERENCES ritmo_checkins (id) ON DELETE CASCADE,
    task_id    INTEGER NOT NULL REFERENCES ritmo_tasks (id) ON DELETE CASCADE,
    status     TEXT NOT NULL CHECK (status IN ('advanced', 'blocked', 'untouched')),
    note       TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (checkin_id, task_id)
);
