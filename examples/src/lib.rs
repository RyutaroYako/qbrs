//! Shared schema + setup helper for the runnable examples in `examples/`.
//! See `README.md` in this directory for setup instructions.

use qbrs::Table;

#[derive(Table)]
#[table(name = "users")]
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
pub struct Orders {
    #[column(primary_key, generated)]
    pub id: i64,
    pub user_id: i64,
    pub total: i64,
    #[column(default)]
    pub shipped: bool,
}

pub use embedded_pg::Db;

/// Connects to `DATABASE_URL` if set, otherwise starts a throwaway embedded
/// Postgres, and resets the schema so every example starts from the same
/// known data. The guard it returns keeps that server alive, so examples
/// bind it (`let (pool, _db) = ...`) rather than discarding it.
pub async fn setup_db() -> (sqlx::PgPool, Db) {
    let (pool, db) = embedded_pg::connect().await;

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

    (pool, db)
}

/// Seeds a small, fixed dataset used by every example: three users (one
/// inactive, one with no orders) and a handful of orders. That is enough to
/// make filters/joins/nullability actually demonstrate something.
pub async fn seed(pool: &sqlx::PgPool) {
    use qbrs::prelude::*;
    use qbrs_sqlx::prelude::*;

    let ids: Vec<i64> = insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("ada@example.com")
                .display_name("Ada Lovelace")
                .build(),
        )
        .values(
            UsersInsert::builder()
                .email("dan@example.com")
                .display_name("Dan")
                .build(),
        )
        .values(
            UsersInsert::builder()
                .email("grace@example.com")
                .display_name("Grace Hopper")
                .build(),
        )
        .returning(users::id)
        .load(pool)
        .await
        .expect("seed users");
    let [ada, dan, grace]: [i64; 3] = ids.try_into().expect("three users seeded");

    update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                active: Some(false),
                ..Default::default()
            })
            .expect("active is set"),
        )
        .filter(users::id.eq(dan))
        .execute(pool)
        .await
        .expect("deactivate dan");

    insert(orders::Table)
        .values(OrdersInsert::builder().user_id(ada).total(1500).build())
        .values(
            OrdersInsert::builder()
                .user_id(ada)
                .total(2500)
                .shipped(true)
                .build(),
        )
        .values(OrdersInsert::builder().user_id(grace).total(999).build())
        .execute(pool)
        .await
        .expect("seed orders");
}
