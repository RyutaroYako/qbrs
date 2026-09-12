//! The throwaway PostgreSQL that the integration tests and the runnable
//! examples both start, and the `initdb`-ed data directory they share.

use std::path::{Path, PathBuf};

/// Keeps an embedded server and its data directory alive for as long as the
/// pool that talks to it. Dropping it stops the server, so a caller binds it
/// (`let (pool, _db) = connect().await;`) rather than discarding it.
pub enum Db {
    External,
    // The server and its data directory both have to outlive the pool, so
    // the guard owns them. Boxed to keep the unit variant small.
    Embedded(Box<(pglite::PGlite, tempfile::TempDir)>),
}

/// Connects to `DATABASE_URL` if set, otherwise starts a throwaway embedded
/// PostgreSQL.
pub async fn connect() -> (sqlx::PgPool, Db) {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let pool = sqlx::PgPool::connect(&url)
            .await
            .unwrap_or_else(|e| panic!("connect to {url}: {e}"));
        return (pool, Db::External);
    }

    let dir = tempfile::tempdir().expect("create temp data dir");
    copy_dir(&initialized_template().await, dir.path()).expect("seed data dir from template");
    // Multi-process mode (a real postmaster over a unix socket): the
    // in-process backend opens once per process and serves one connection,
    // which would cap every pool at 1.
    let db = pglite::PGlite::open_multi_process(dir.path(), pglite::MultiProcessOptions::default())
        .await
        .expect("start embedded postgres");
    let url = db.unix_uri().await.expect("embedded postgres socket uri");
    let pool = sqlx::PgPool::connect(&url)
        .await
        .unwrap_or_else(|e| panic!("connect to embedded postgres at {url}: {e}"));
    (pool, Db::Embedded(Box::new((db, dir))))
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

/// A data directory `initdb` has already run in, built once per `target/`
/// and copied per caller. `open_multi_process` skips `initdb` when
/// `PG_VERSION` is already there, which is the whole saving: bootstrapping
/// the template databases costs seconds, copying the result milliseconds.
async fn initialized_template() -> PathBuf {
    let template = target_tmp_dir().join("pg-template");
    if template.join("PG_VERSION").exists() {
        return template;
    }
    // A run killed mid-bootstrap leaves a directory `initdb` then refuses to
    // touch, and nothing can be copying from one with no `PG_VERSION` in it.
    let _ = std::fs::remove_dir_all(&template);

    let staged = template.with_file_name(format!("pg-template.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staged);
    std::fs::create_dir_all(&staged).expect("create staging data dir");
    let db = pglite::PGlite::open_multi_process(&staged, pglite::MultiProcessOptions::default())
        .await
        .expect("initialize template data dir");
    // A clean shutdown is what leaves no `postmaster.pid` behind for the
    // copies to trip over.
    db.close().await.ok();

    // Renaming onto a directory that already holds a template fails rather
    // than replacing it, which is what keeps concurrent test binaries from
    // publishing half of one over another's.
    if std::fs::rename(&staged, &template).is_err() {
        let _ = std::fs::remove_dir_all(&staged);
    }
    template
}

/// `<target-dir>/tmp`, which cargo hands an integration test as
/// `CARGO_TARGET_TMPDIR` and hands an example nothing at all — so the path
/// is read off the running binary, two directories below the profile dir
/// either way (`<target>/debug/deps/` and `<target>/debug/examples/`).
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
