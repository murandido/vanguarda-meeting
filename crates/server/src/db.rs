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

/// Cria ou atualiza o usuário no login. `login` já vem em minúsculas.
pub async fn upsert_user(
    db: &SqlitePool,
    login: &str,
    github_id: Option<i64>,
    name: Option<&str>,
    avatar_url: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO users (login, github_id, name, avatar_url) VALUES (?, ?, ?, ?)
         ON CONFLICT (login) DO UPDATE
         SET github_id = COALESCE(excluded.github_id, users.github_id),
             name = COALESCE(excluded.name, users.name),
             avatar_url = COALESCE(excluded.avatar_url, users.avatar_url),
             last_login_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        login,
        github_id,
        name,
        avatar_url
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn create_session(
    db: &SqlitePool,
    id: &str,
    login: &str,
    expires_at: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO sessions (id, login, expires_at) VALUES (?, ?, ?)",
        id,
        login,
        expires_at
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Login dono da sessão, se ela existir e não tiver expirado.
pub async fn session_login(db: &SqlitePool, id: &str, now: i64) -> sqlx::Result<Option<String>> {
    let row = sqlx::query!(
        "SELECT login FROM sessions WHERE id = ? AND expires_at > ?",
        id,
        now
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.login))
}

pub async fn delete_session(db: &SqlitePool, id: &str) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM sessions WHERE id = ?", id)
        .execute(db)
        .await?;
    Ok(())
}
