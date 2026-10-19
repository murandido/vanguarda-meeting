use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::str::FromStr;

/// Abre (criando se preciso) o banco e aplica as migrações.
pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    // Em memória, cada conexão seria um banco diferente: fica uma só.
    let max = if url.contains(":memory:") { 1 } else { 8 };
    let pool = SqlitePoolOptions::new()
        .max_connections(max)
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub async fn load_snapshot(db: &SqlitePool, board_id: &str) -> sqlx::Result<Option<Vec<u8>>> {
    let row = sqlx::query!(
        "SELECT snapshot FROM board_snapshots WHERE board_id = ?",
        board_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.snapshot))
}

pub async fn save_snapshot(db: &SqlitePool, board_id: &str, snapshot: &[u8]) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO board_snapshots (board_id, snapshot) VALUES (?, ?)
         ON CONFLICT (board_id) DO UPDATE
         SET snapshot = excluded.snapshot,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        board_id,
        snapshot
    )
    .execute(db)
    .await?;
    Ok(())
}
