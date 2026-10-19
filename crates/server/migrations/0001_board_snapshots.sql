-- Último snapshot Loro de cada quadro do Cartazes.
CREATE TABLE board_snapshots (
    board_id   TEXT PRIMARY KEY NOT NULL,
    snapshot   BLOB NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
