//! Shared setup for real-Postgres integration tests.

pub use embedded_pg::{Db as PgGuard, shutdown};

pub async fn test_pool(_db_name: &str) -> (sqlx::PgPool, PgGuard) {
    embedded_pg::connect().await
}
