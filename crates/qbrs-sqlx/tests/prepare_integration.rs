//! `prepare!{}` executed against a real Postgres: one rendered query,
//! reused across multiple `.load(executor, params)` calls with different values.
//! See `postgres_integration.rs` for the DB-setup rationale (in-process
//! WASM Postgres vs. an external `DATABASE_URL`).

mod common;

use qbrs::dialect::Postgres;
use qbrs::expr::{ExprMethods, Text};
use qbrs::select::select;
use qbrs::statement::Statement;
use qbrs::{Table, prepare};
use qbrs_sqlx::{LoadExt, PreparedExt};

#[derive(Table)]
#[table(name = "users_prepare_test")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
}

prepare! {
    struct ByEmail { email: Text }
}

#[tokio::test]
async fn prepared_query_reused_across_different_params() {
    let (pool, guard) = common::test_pool("qbrs_test_prepare").await;

    sqlx::query("DROP TABLE IF EXISTS users_prepare_test")
        .execute(&pool)
        .await
        .expect("drop table");
    sqlx::query(
        "CREATE TABLE users_prepare_test (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            email TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("create table");

    let ids: Vec<i64> = qbrs::insert::insert(users::Table)
        .values(UsersInsert::builder().email("ada@example.com").build())
        .values(UsersInsert::builder().email("dan@example.com").build())
        .returning(users::id)
        .load(&pool)
        .await
        .expect("seed users");
    assert_eq!(ids.len(), 2);

    let query = select(users::id)
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare::<ByEmail, _>(Postgres);

    let ada: Vec<i64> = query
        .load(
            &pool,
            ByEmail {
                email: "ada@example.com".to_string(),
            },
        )
        .await
        .expect("load for ada");
    assert_eq!(ada, vec![ids[0]]);

    // Same `query` value, no re-render — just a different `params`.
    let dan: Vec<i64> = query
        .load(
            &pool,
            ByEmail {
                email: "dan@example.com".to_string(),
            },
        )
        .await
        .expect("load for dan");
    assert_eq!(dan, vec![ids[1]]);

    sqlx::query("DROP TABLE users_prepare_test")
        .execute(&pool)
        .await
        .expect("cleanup");
    common::shutdown(pool, guard).await;
}
