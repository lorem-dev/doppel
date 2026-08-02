//! `doppel config migrate`.

use doppel_store_postgres::migrate as apply;
use sqlx::{Connection, PgConnection};

use crate::cli::{CliError, MigrateArgs, mask_dsn};

/// Apply the embedded migrations, reporting how many the database already had.
///
/// The count comes from asking before and after rather than from what the
/// migrator says it did, so "already up to date" and "applied three" are
/// distinguishable to whoever ran the command -- which is the difference
/// between reassurance and a silent no-op.
pub async fn migrate(args: &MigrateArgs) -> Result<String, CliError> {
    // One connection, not a pool. A pool retries until its acquire timeout,
    // so a refused connection would sit silent for thirty seconds before
    // reporting what the operating system said immediately.
    let mut conn = PgConnection::connect(&args.database_url)
        .await
        .map_err(|err| {
            CliError::Failed(format!(
                "cannot reach the database at {}: {err}",
                mask_dsn(&args.database_url)
            ))
        })?;

    let before = applied_count(&mut conn).await;
    apply(&mut conn)
        .await
        .map_err(|err| CliError::Failed(err.to_string()))?;
    let after = applied_count(&mut conn).await;
    let _ = conn.close().await;

    Ok(match after.saturating_sub(before) {
        0 => format!("already up to date; {} applied", plural(after)),
        applied => format!("applied {}; {} in total", plural(applied), plural(after)),
    })
}

/// "1 migration" or "3 migrations". Worth the three lines: this string is the
/// command's whole output, and an operator reading "1 migrations" learns that
/// nobody looked at it.
fn plural(count: u64) -> String {
    if count == 1 {
        "1 migration".to_owned()
    } else {
        format!("{count} migrations")
    }
}

/// How many migrations the database records. Zero when the bookkeeping table
/// does not exist yet, which is what an untouched database looks like.
async fn applied_count(conn: &mut PgConnection) -> u64 {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&mut *conn)
        .await
        .unwrap_or(0);
    u64::try_from(count).unwrap_or(0)
}
