//! Shared setup for real-Postgres integration tests.

/// Either connects to `DATABASE_URL` directly (external-Postgres path) or
/// starts a throwaway embedded Postgres (default path), returning a pool
/// plus a guard that shuts the embedded instance down.
pub enum PgGuard {
    External,
    // The server and its data directory both have to outlive the pool, so
    // the guard owns them. Boxed to keep the unit variant small.
    Embedded(Box<(pglite::PGlite, tempfile::TempDir)>),
}

pub async fn test_pool(_db_name: &str) -> (sqlx::PgPool, PgGuard) {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let pool = sqlx::PgPool::connect(&url)
            .await
            .expect("connect to DATABASE_URL");
        return (pool, PgGuard::External);
    }

    let dir = tempfile::tempdir().expect("create temp data dir");
    // Multi-process mode (a real postmaster over a unix socket): the
    // in-process backend opens once per process and serves one connection,
    // which would cap every pool at 1.
    let db = pglite::PGlite::open_multi_process(dir.path(), pglite::MultiProcessOptions::default())
        .await
        .expect("start embedded postgres");
    let url = db.unix_uri().await.expect("embedded postgres socket uri");
    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to embedded postgres");
    (pool, PgGuard::Embedded(Box::new((db, dir))))
}

pub async fn shutdown(pool: sqlx::PgPool, guard: PgGuard) {
    pool.close().await;
    if let PgGuard::Embedded(embedded) = guard {
        // The postmaster is a child process: without this it outlives the
        // test binary.
        let (db, dir) = *embedded;
        db.close().await.ok();
        drop(dir);
    }
}
