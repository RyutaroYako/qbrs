//! Shared setup for real-Postgres integration tests. `tests/common/mod.rs`
//! (rather than a shared library crate) is the standard way to share code
//! between `cargo test` integration-test binaries without publishing it as
//! part of the crate's own API surface — each test binary that does `mod
//! common;` gets its own compiled copy, but the *source* isn't duplicated.

/// Either connects to `DATABASE_URL` directly (external-Postgres path) or
/// starts a throwaway WASM Postgres (default path), returning a pool plus a
/// guard that shuts the embedded instance down on drop.
pub enum PgGuard {
    External,
    // Boxed for the same reason the old `postgresql_embedded` guard was:
    // the running-server variant is far larger than the unit `External`
    // one, and clippy's `large_enum_variant` lint is right that leaving it
    // unboxed makes every `PgGuard` pay for the biggest variant's size.
    Wasm(Box<pglite_oxide::PgliteServer>),
}

pub async fn test_pool(_db_name: &str) -> (sqlx::PgPool, PgGuard) {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let pool = sqlx::PgPool::connect(&url)
            .await
            .expect("connect to DATABASE_URL");
        return (pool, PgGuard::External);
    }

    let server = pglite_oxide::PgliteServer::temporary_tcp().expect("start WASM postgres");
    // The WASIX Postgres backend is a single-backend build: it accepts
    // exactly one client connection at a time, and a second concurrent
    // connect just hangs until the first is released. sqlx's default pool
    // (max 10) will happily try to open a second one and dead-lock itself,
    // so the pool has to be capped at 1.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&server.database_url())
        .await
        .expect("connect to WASM postgres");
    (pool, PgGuard::Wasm(Box::new(server)))
}

pub async fn shutdown(pool: sqlx::PgPool, guard: PgGuard) {
    pool.close().await;
    if let PgGuard::Wasm(server) = guard {
        server.shutdown().ok();
    }
}
