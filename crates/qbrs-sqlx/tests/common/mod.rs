//! Shared setup for real-Postgres integration tests. `tests/common/mod.rs`
//! (rather than a shared library crate) is the standard way to share code
//! between `cargo test` integration-test binaries without publishing it as
//! part of the crate's own API surface — each test binary that does `mod
//! common;` gets its own compiled copy, but the *source* isn't duplicated.

/// Either connects to `DATABASE_URL` directly (external-Postgres path) or
/// starts a throwaway embedded Postgres (default path), returning a pool
/// plus a guard that shuts the embedded instance down.
pub enum PgGuard {
    External,
    // Both the server and its data directory have to outlive the pool, so
    // the guard owns them. Boxed for the same reason the previous guard
    // was: the running-server variant is far larger than the unit
    // `External` one, and clippy's `large_enum_variant` lint is right that
    // leaving it unboxed makes every `PgGuard` pay the biggest variant's
    // size.
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
    // Multi-process mode (a real postmaster over a unix socket) rather than
    // the single in-process backend: the in-process one can only ever be
    // opened once per process and serves a single connection, so it would
    // cap every pool at 1 and quietly break the moment a second real-DB
    // test lands in the same test binary.
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
        // Not just tidiness: the postmaster is a child process, so skipping
        // this would leave it (and its workers) running after the test
        // binary exits.
        let (db, dir) = *embedded;
        db.close().await.ok();
        drop(dir);
    }
}
