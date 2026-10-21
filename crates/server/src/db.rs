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

/// Salva o snapshot e marca o quadro como atualizado agora.
pub async fn save_snapshot(db: &SqlitePool, board_id: &str, snapshot: &[u8]) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query!(
        "INSERT INTO board_snapshots (board_id, snapshot) VALUES (?, ?)
         ON CONFLICT (board_id) DO UPDATE
         SET snapshot = excluded.snapshot,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        board_id,
        snapshot
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE boards SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        board_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Um quadro do Cartazes (tabela `boards`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardRow {
    pub id: String,
    pub title: String,
    pub created_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

impl BoardRow {
    #[must_use]
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }
}

/// Todos os quadros, do atualizado mais recentemente para o mais antigo.
pub async fn list_boards(db: &SqlitePool) -> sqlx::Result<Vec<BoardRow>> {
    sqlx::query_as!(
        BoardRow,
        "SELECT id, title, created_by, created_at, updated_at, archived_at
         FROM boards ORDER BY updated_at DESC, id"
    )
    .fetch_all(db)
    .await
}

pub async fn get_board(db: &SqlitePool, id: &str) -> sqlx::Result<Option<BoardRow>> {
    sqlx::query_as!(
        BoardRow,
        "SELECT id, title, created_by, created_at, updated_at, archived_at
         FROM boards WHERE id = ?",
        id
    )
    .fetch_optional(db)
    .await
}

/// Cria o quadro; devolve `false` se o id já existir.
pub async fn create_board(
    db: &SqlitePool,
    id: &str,
    title: &str,
    created_by: &str,
) -> sqlx::Result<bool> {
    let done = sqlx::query!(
        "INSERT INTO boards (id, title, created_by) VALUES (?, ?, ?)
         ON CONFLICT (id) DO NOTHING",
        id,
        title,
        created_by
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Renomeia; devolve `false` se o quadro não existir.
pub async fn rename_board(db: &SqlitePool, id: &str, title: &str) -> sqlx::Result<bool> {
    let done = sqlx::query!(
        "UPDATE boards SET title = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
        title,
        id
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Arquiva ou desarquiva; devolve `false` se o quadro não existir.
pub async fn set_board_archived(db: &SqlitePool, id: &str, archived: bool) -> sqlx::Result<bool> {
    let done = sqlx::query!(
        "UPDATE boards
         SET archived_at = CASE WHEN ? THEN COALESCE(archived_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) END
         WHERE id = ?",
        archived,
        id
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Creates or updates the user on a development login. `login` is already
/// lowercase. A stored `github_id` is never overwritten.
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
         SET github_id = COALESCE(users.github_id, excluded.github_id),
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

/// Outcome of [`login_github_user`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubLogin {
    /// The account is bound to the login (now or before): let it in.
    Accepted,
    /// The login already belongs to another GitHub account: refuse.
    Conflict,
}

/// Binds the GitHub account `github_id` to the invited (lowercase) `login`,
/// trusting the first use: the first `github_id` seen for a login is stored
/// and never overwritten.
///
/// - Login stored with another `github_id` → [`GithubLogin::Conflict`]
///   (someone registered a login that used to belong to somebody else).
/// - `github_id` stored under another login → the account was renamed on
///   GitHub (and the new name is invited): the row is renamed and the old
///   name's sessions are deleted.
pub async fn login_github_user(
    db: &SqlitePool,
    login: &str,
    github_id: i64,
    name: Option<&str>,
    avatar_url: Option<&str>,
) -> sqlx::Result<GithubLogin> {
    let mut tx = db.begin().await?;
    let by_id = sqlx::query_scalar!("SELECT login FROM users WHERE github_id = ?", github_id)
        .fetch_optional(&mut *tx)
        .await?;
    let by_login = sqlx::query_scalar!("SELECT github_id FROM users WHERE login = ?", login)
        .fetch_optional(&mut *tx)
        .await?;

    match (by_id, by_login) {
        (Some(old), _) if old == login => {}
        // The new name already belongs to another GitHub account.
        (_, Some(Some(_))) => return Ok(GithubLogin::Conflict),
        (Some(old), existing) => {
            if existing.is_some() {
                // A row without `github_id` (development login) makes way
                // for the real account.
                sqlx::query!("DELETE FROM users WHERE login = ?", login)
                    .execute(&mut *tx)
                    .await?;
            }
            // `sessions.login` references `users.login` without ON UPDATE:
            // the old name's sessions go before the rename.
            sqlx::query!("DELETE FROM sessions WHERE login = ?", old)
                .execute(&mut *tx)
                .await?;
            sqlx::query!("UPDATE users SET login = ? WHERE login = ?", login, old)
                .execute(&mut *tx)
                .await?;
        }
        (None, Some(None)) => {
            sqlx::query!(
                "UPDATE users SET github_id = ? WHERE login = ?",
                github_id,
                login
            )
            .execute(&mut *tx)
            .await?;
        }
        (None, None) => {
            sqlx::query!(
                "INSERT INTO users (login, github_id) VALUES (?, ?)",
                login,
                github_id
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    sqlx::query!(
        "UPDATE users
         SET name = COALESCE(?, name),
             avatar_url = COALESCE(?, avatar_url),
             last_login_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE login = ?",
        name,
        avatar_url,
        login
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(GithubLogin::Accepted)
}

/// The `github_id` stored for `login`, if any.
pub async fn github_id_of(db: &SqlitePool, login: &str) -> sqlx::Result<Option<i64>> {
    let id = sqlx::query_scalar!("SELECT github_id FROM users WHERE login = ?", login)
        .fetch_optional(db)
        .await?;
    Ok(id.flatten())
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
