//! Shared schema + setup helper for the runnable examples in `examples/`.
//! See `README.md` in this directory for setup instructions.

use qbrs::Table;

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
pub struct Users {
    #[column(primary_key, generated)]
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    #[column(default)]
    pub active: bool,
}

#[derive(Table)]
#[table(name = "orders")]
#[allow(dead_code)]
pub struct Orders {
    #[column(primary_key, generated)]
    pub id: i64,
    pub user_id: i64,
    pub total: i64,
    #[column(default)]
    pub shipped: bool,
}

/// Connects to `DATABASE_URL` if set, otherwise starts a throwaway WASM
/// Postgres, and resets the schema so every example starts from the same
/// known data.
pub async fn setup_db() -> sqlx::PgPool {
    let pool = match std::env::var("DATABASE_URL") {
        Ok(url) => sqlx::PgPool::connect(&url)
            .await
            .unwrap_or_else(|e| panic!("connect to {url}: {e}")),
        Err(_) => {
            let server = pglite_oxide::PgliteServer::temporary_tcp().expect("start WASM postgres");
            let url = server.database_url();
            // Each example is a short-lived process that wants the server up
            // until it exits, so the server is leaked rather than threaded
            // back through every example's `main` as a guard. The temporary
            // data directory is cleaned up by the OS, not by us.
            std::mem::forget(server);
            // The WASIX Postgres backend accepts exactly one client
            // connection at a time; sqlx's default pool (max 10) would try
            // to open a second one and dead-lock, so it is capped at 1.
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect(&url)
                .await
                .unwrap_or_else(|e| panic!("connect to WASM postgres at {url}: {e}"))
        }
    };

    sqlx::query("DROP TABLE IF EXISTS orders")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS users")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE users (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            email TEXT NOT NULL UNIQUE,
            display_name TEXT,
            active BOOLEAN NOT NULL DEFAULT true
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TABLE orders (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            user_id BIGINT NOT NULL REFERENCES users(id),
            total BIGINT NOT NULL,
            shipped BOOLEAN NOT NULL DEFAULT false
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    pool
}

/// Seeds a small, fixed dataset used by every example: three users (one
/// inactive, one with no orders) and a handful of orders — enough to make
/// filters/joins/nullability actually demonstrate something.
pub async fn seed(pool: &sqlx::PgPool) {
    use qbrs::dialect::Postgres;
    use qbrs::expr::ExprMethods;
    use qbrs_sqlx::{ExecuteExt, LoadReturningExt};

    let ids: Vec<i64> = qbrs::insert::insert::<Postgres, _>(users::Table)
        .values(UsersInsert::new("ada@example.com").display_name("Ada Lovelace"))
        .values(UsersInsert::new("dan@example.com").display_name("Dan"))
        .values(UsersInsert::new("grace@example.com").display_name("Grace Hopper"))
        .returning(users::id)
        .load(pool)
        .await
        .expect("seed users");

    // Dan (ids[1]) intentionally gets no orders and active=false, so the
    // examples have something interesting to filter/outer-join against.
    qbrs::update::update::<Postgres, _>(users::Table)
        .set(UsersUpdate {
            active: Some(false),
            ..Default::default()
        })
        .filter(users::id.eq(ids[1]))
        .execute(pool)
        .await
        .expect("deactivate dan");

    qbrs::insert::insert::<Postgres, _>(orders::Table)
        .values(OrdersInsert::new(ids[0], 1500))
        .values(OrdersInsert::new(ids[0], 2500).shipped(true))
        .values(OrdersInsert::new(ids[2], 999))
        .execute(pool)
        .await
        .expect("seed orders");
}
