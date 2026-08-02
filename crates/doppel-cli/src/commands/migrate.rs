//! `doppel config migrate`.

use doppel_store_postgres::{MIGRATOR, migrate as apply};
use sqlx::{Connection, PgConnection};

use crate::cli::{CliError, MigrateArgs, mask_dsn};

/// What a run of this command has to say, and the code it exits with.
///
/// A struct rather than a bare `String` because `--status` reports a state
/// rather than an outcome, and "behind" is a state a deploy gate needs to
/// branch on. Returning text alone would have made the only way to say
/// "behind" an `Err`, which reads as a failure of the command rather than a
/// finding about the database.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub text: String,
    pub code: u8,
}

impl Report {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            code: 0,
        }
    }

    fn behind(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            code: 1,
        }
    }
}

/// Apply the embedded migrations, or report what is applied without touching
/// anything.
pub async fn migrate(args: &MigrateArgs) -> Result<Report, CliError> {
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

    let report = if args.status {
        status(&mut conn).await
    } else {
        apply_all(&mut conn).await
    };

    let _ = conn.close().await;
    report
}

/// Apply, reporting how many the database already had.
///
/// The count comes from asking before and after rather than from what the
/// migrator says it did, so "already up to date" and "applied three" are
/// distinguishable to whoever ran the command -- which is the difference
/// between reassurance and a silent no-op.
async fn apply_all(conn: &mut PgConnection) -> Result<Report, CliError> {
    let before = applied(conn).await.map_or(0, |rows| rows.len());
    apply(&mut *conn)
        .await
        .map_err(|err| CliError::Failed(err.to_string()))?;
    let after = applied(conn).await.map_or(0, |rows| rows.len());

    Ok(Report::ok(match after.saturating_sub(before) {
        0 => format!("already up to date; {} applied", plural(after)),
        applied => format!("applied {}; {} in total", plural(applied), plural(after)),
    }))
}

/// Report the schema state without changing it.
///
/// Three things an operator or a deploy gate wants to know, and none of them
/// is the row count: which version the database is at, whether every
/// migration this binary carries is applied, and whether any applied one has
/// been edited since. The last is why this reads sqlx's table rather than a
/// single stored revision number -- a number cannot notice that the file
/// behind it changed.
async fn status(conn: &mut PgConnection) -> Result<Report, CliError> {
    let Some(rows) = applied(conn).await else {
        return Ok(Report::behind(
            "no migrations have been applied; run `doppel config migrate`",
        ));
    };

    let mut lines = match rows.iter().map(|row| row.version).max() {
        Some(version) => vec![format!(
            "schema version {version}; {} applied",
            plural(rows.len())
        )],
        None => vec!["schema version none; 0 migrations applied".to_owned()],
    };

    let mut behind = false;
    for migration in MIGRATOR.iter() {
        match rows.iter().find(|row| row.version == migration.version) {
            None => {
                behind = true;
                lines.push(format!(
                    "missing: migration {} ({})",
                    migration.version, migration.description
                ));
            }
            Some(row) if !row.success => {
                behind = true;
                lines.push(format!(
                    "failed: migration {} ({}) is recorded as not having completed",
                    migration.version, migration.description
                ));
            }
            Some(row) if row.checksum != *migration.checksum => {
                // sqlx refuses to run against this, and so should anything
                // else: the file that produced the schema is not the file
                // this binary carries, so nobody knows what the schema is.
                behind = true;
                lines.push(format!(
                    "changed: migration {} ({}) does not match the file this binary carries",
                    migration.version, migration.description
                ));
            }
            Some(_) => {}
        }
    }

    // Applied migrations this binary knows nothing about. Not a failure of
    // this binary's own requirements, so it does not set `behind`, but worth
    // saying: it usually means an older binary is looking at a database a
    // newer one has already migrated.
    for row in &rows {
        if !MIGRATOR.iter().any(|m| m.version == row.version) {
            lines.push(format!(
                "unknown: migration {} ({}) is applied but not carried by this binary",
                row.version, row.description
            ));
        }
    }

    if behind {
        lines.push("run `doppel config migrate`".to_owned());
        Ok(Report::behind(lines.join("\n")))
    } else {
        lines.push("up to date".to_owned());
        Ok(Report::ok(lines.join("\n")))
    }
}

/// One row of sqlx's bookkeeping table.
struct Applied {
    version: i64,
    description: String,
    checksum: Vec<u8>,
    success: bool,
}

/// The applied migrations, or `None` when the bookkeeping table does not
/// exist -- which is what an untouched database looks like.
///
/// A missing table and a failed query are not distinguished, deliberately:
/// both mean this command cannot see any bookkeeping, and both are answered
/// by running the migration, which says something more specific if the real
/// problem is something else.
async fn applied(conn: &mut PgConnection) -> Option<Vec<Applied>> {
    let rows: Vec<(i64, String, Vec<u8>, bool)> = sqlx::query_as(
        "SELECT version, description, checksum, success FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&mut *conn)
    .await
    .ok()?;

    Some(
        rows.into_iter()
            .map(|(version, description, checksum, success)| Applied {
                version,
                description,
                checksum,
                success,
            })
            .collect(),
    )
}

/// "1 migration" or "3 migrations". Worth the three lines: this string is the
/// command's whole output, and an operator reading "1 migrations" learns that
/// nobody looked at it.
fn plural(count: usize) -> String {
    if count == 1 {
        "1 migration".to_owned()
    } else {
        format!("{count} migrations")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plural_reads_like_something_a_person_wrote() {
        assert_eq!(plural(0), "0 migrations");
        assert_eq!(plural(1), "1 migration");
        assert_eq!(plural(2), "2 migrations");
    }

    #[test]
    fn a_report_carries_its_exit_code() {
        // The reason this is a struct: "behind" is a state a deploy gate
        // branches on, not a failure of the command.
        assert_eq!(Report::ok("fine").code, 0);
        assert_eq!(Report::behind("not fine").code, 1);
    }
}
