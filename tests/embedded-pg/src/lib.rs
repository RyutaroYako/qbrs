//! The throwaway PostgreSQL that the integration tests and the runnable
//! examples both start, and the `initdb`-ed data directory they share.

use std::path::{Path, PathBuf};

/// Keeps an embedded server and its data directory alive for as long as the
/// pool that talks to it. Dropping it stops the server, so a caller binds it
/// (`let (pool, _db) = connect().await;`) rather than discarding it.
pub enum Db {
    External,
    Embedded(Box<(pglite::PGlite, tempfile::TempDir)>),
}

/// Connects to `DATABASE_URL` if set, otherwise starts a throwaway embedded
/// PostgreSQL.
pub async fn connect() -> (sqlx::PgPool, Db) {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        // The message names the variable rather than including the URL, which carries
        // credentials.
        let pool = sqlx::PgPool::connect(&url)
            .await
            .unwrap_or_else(|e| panic!("connect to DATABASE_URL: {e}"));
        return (pool, Db::External);
    }

    if let Ok(started) = start_from(&template().await).await {
        return started;
    }
    // Rebuilding rather than reporting, since the template is only a cache of `initdb`. A
    // copy fails to start when a `pglite-rs` bump moved the PostgreSQL major, or when
    // another run's rebuild deleted the template mid-copy.
    let _ = std::fs::remove_dir_all(template_path());
    start_from(&template().await)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
}

/// Closes the pool and stops the server. A test binary needs this: the
/// postmaster is a child process and outlives the binary without it.
pub async fn shutdown(pool: sqlx::PgPool, db: Db) {
    pool.close().await;
    if let Db::Embedded(embedded) = db {
        let (db, dir) = *embedded;
        db.close().await.ok();
        drop(dir);
    }
}

async fn start_from(template: &Path) -> Result<(sqlx::PgPool, Db), String> {
    let dir = tempfile::tempdir().expect("create temp data dir");
    copy_dir(template, dir.path()).map_err(|e| format!("seed data dir from template: {e}"))?;
    // Not the in-process backend, which opens once per process and serves one connection,
    // capping every pool at 1.
    let db = pglite::PGlite::open_multi_process(dir.path(), pglite::MultiProcessOptions::default())
        .await
        .map_err(|e| format!("start embedded postgres: {e}"))?;
    let url = db
        .unix_uri()
        .await
        .map_err(|e| format!("embedded postgres socket uri: {e}"))?;
    let pool = sqlx::PgPool::connect(&url)
        .await
        .map_err(|e| format!("connect to embedded postgres at {url}: {e}"))?;
    Ok((pool, Db::Embedded(Box::new((db, dir)))))
}

async fn template() -> PathBuf {
    let template = template_path();
    if initdb_has_run_in(&template) {
        return template;
    }

    let staged = template.with_file_name(format!("pg-template.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staged);
    std::fs::create_dir_all(&staged).expect("create staging data dir");
    let db = pglite::PGlite::open_multi_process(&staged, pglite::MultiProcessOptions::default())
        .await
        .expect("initialize template data dir");
    // Skipping this clean shutdown leaves a `postmaster.pid` in the template, which every
    // copy then inherits.
    db.close().await.ok();

    publish(&staged, &template);
    template
}

// No lock is needed: renaming onto a directory that already holds a template fails rather
// than replacing it, so the first run to finish wins and the rest discard their staged copy.
// `initdb` runs in `staged`, so the shared path never holds a template `initdb` has not
// finished.
fn publish(staged: &Path, template: &Path) {
    if std::fs::rename(staged, template).is_ok() {
        return;
    }
    if !initdb_has_run_in(template) {
        let _ = std::fs::remove_dir_all(template);
        if std::fs::rename(staged, template).is_ok() {
            return;
        }
    }
    let _ = std::fs::remove_dir_all(staged);
}

fn initdb_has_run_in(dir: &Path) -> bool {
    dir.join("PG_VERSION").exists()
}

fn template_path() -> PathBuf {
    target_tmp_dir().join("pg-template")
}

// Not `CARGO_TARGET_TMPDIR`, which cargo sets for an integration test but not for an
// example. Both binaries sit two directories below `<target>` (`debug/deps/`,
// `debug/examples/`), so the path is read off the running binary.
fn target_tmp_dir() -> PathBuf {
    std::env::current_exe()
        .expect("path of the running binary")
        .ancestors()
        .nth(3)
        .expect("target directory above the running binary")
        .join("tmp")
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    std::fs::set_permissions(dst, std::fs::metadata(src)?.permissions())?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}
