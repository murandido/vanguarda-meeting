-- Usuários convidados (identificados pelo login do GitHub, em minúsculas).
CREATE TABLE users (
    login         TEXT PRIMARY KEY NOT NULL,
    github_id     INTEGER UNIQUE,
    name          TEXT,
    avatar_url    TEXT,
    last_login_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Sessões de login. `expires_at` em segundos Unix.
CREATE TABLE sessions (
    id         TEXT PRIMARY KEY NOT NULL,
    login      TEXT NOT NULL REFERENCES users (login) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);

CREATE INDEX sessions_login ON sessions (login);
