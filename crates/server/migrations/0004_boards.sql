-- Lista de quadros do Cartazes (CA-09). Antes daqui, qualquer `/b/{id}`
-- criava um quadro; agora só existem os quadros desta tabela.
CREATE TABLE boards (
    id          TEXT PRIMARY KEY NOT NULL,
    title       TEXT NOT NULL,
    -- Login de quem criou; NULL nos quadros anteriores a esta migração.
    created_by  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    archived_at TEXT
);

CREATE INDEX boards_updated_at ON boards (updated_at);

-- Os quadros que já têm snapshot continuam abrindo: o título inicial é o id.
INSERT INTO boards (id, title, created_at, updated_at)
SELECT board_id, board_id, updated_at, updated_at FROM board_snapshots;
