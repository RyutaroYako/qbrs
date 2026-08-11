//! Shared setup for real-Postgres integration tests. `tests/common/mod.rs`
//! (rather than a shared library crate) is the standard way to share code
//! between `cargo test` integration-test binaries without publishing it as
//! part of the crate's own API surface — each test binary that does `mod
//! common;` gets its own compiled copy, but the *source* isn't duplicated.

/// Either connects to `DATABASE_URL` directly (docker-compose path) or
/// provisions a throwaway embedded Postgres (no-Docker path), returning a
/// pool plus a guard that shuts the embedded instance down on drop.
pub enum PgGuard {
    External,
    // Boxed: `postgresql_embedded::PostgreSQL` is much larger than the
    // unit `External` variant, and clippy's `large_enum_variant` lint is
    // right that leaving it unboxed would make every `PgGuard` pay for the
    // biggest variant's size even when it's `External`.
    Embedded(Box<postgresql_embedded::PostgreSQL>),
}

pub async fn test_pool(db_name: &str) -> (sqlx::PgPool, PgGuard) {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let pool = sqlx::PgPool::connect(&url)
            .await
            .expect("connect to DATABASE_URL");
        return (pool, PgGuard::External);
    }

    let mut pg = postgresql_embedded::PostgreSQL::default();
    pg.setup().await.expect(
        "download/setup embedded postgres (no DATABASE_URL set, and this network environment \
         can't reach the embedded-binary download host — try \
         `cd examples && docker compose up -d` and set \
         DATABASE_URL=postgres://postgres:postgres@localhost:55432/qbrs_test instead)",
    );
    pg.start().await.expect("start embedded postgres");
    pg.create_database(db_name).await.expect("create database");
    let url = pg.settings().url(db_name);
    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to embedded postgres");
    (pool, PgGuard::Embedded(Box::new(pg)))
}

pub async fn shutdown(pool: sqlx::PgPool, guard: PgGuard) {
    pool.close().await;
    if let PgGuard::Embedded(pg) = guard {
        pg.stop().await.ok();
    }
}
