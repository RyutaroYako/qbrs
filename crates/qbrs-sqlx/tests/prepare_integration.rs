mod common;

use qbrs::dialect::Postgres;
use qbrs::expr::{ExprMethods, Text};
use qbrs::select::select;
use qbrs::statement::Statement;
use qbrs::{Table, prepare};
use qbrs_sqlx::{LoadExt, PreparedExt, StreamExt as _};
use sqlx::PgPool;

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

fn by_email(email: &str) -> ByEmail {
    ByEmail {
        email: email.to_string(),
    }
}

#[tokio::test]
async fn a_prepared_query_is_rendered_once_and_run_with_whatever_params_each_call_brings() {
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
    let [ada_id, dan_id] = ids[..] else {
        panic!("expected two ids, got {ids:?}");
    };

    one_prepared_query_loads_a_different_row_for_each_params_value(&pool, ada_id, dan_id).await;
    a_placeholder_named_in_two_clauses_binds_as_one_parameter(&pool, ada_id).await;
    a_prepared_query_streams_with_the_params_given_at_the_call(&pool, dan_id).await;

    sqlx::query("DROP TABLE users_prepare_test")
        .execute(&pool)
        .await
        .expect("cleanup");
    common::shutdown(pool, guard).await;
}

async fn one_prepared_query_loads_a_different_row_for_each_params_value(
    pool: &PgPool,
    ada_id: i64,
    dan_id: i64,
) {
    let query = select(users::id)
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare::<ByEmail, _>(Postgres);

    let ada: Vec<i64> = query
        .load(pool, by_email("ada@example.com"))
        .await
        .expect("load for ada");
    assert_eq!(ada, vec![ada_id]);

    let dan: Vec<i64> = query
        .load(pool, by_email("dan@example.com"))
        .await
        .expect("load for dan");
    assert_eq!(dan, vec![dan_id]);
}

async fn a_placeholder_named_in_two_clauses_binds_as_one_parameter(pool: &PgPool, ada_id: i64) {
    let twice = select(users::id)
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .filter(users::email.gte(ByEmail::email()))
        .prepare::<ByEmail, _>(Postgres);
    let (sql, params) = twice
        .resolve(by_email("ada@example.com"))
        .expect("every placeholder resolved");
    assert!(
        sql.ends_with(
            r#"WHERE ("users_prepare_test"."email" = $1) AND ("users_prepare_test"."email" >= $1)"#
        ),
        "{sql}"
    );
    assert_eq!(params.len(), 1);

    let ada_again: Vec<i64> = twice
        .load(pool, by_email("ada@example.com"))
        .await
        .expect("load through a placeholder named twice");
    assert_eq!(ada_again, vec![ada_id]);
}

async fn a_prepared_query_streams_with_the_params_given_at_the_call(pool: &PgPool, dan_id: i64) {
    let query = select(users::id)
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare::<ByEmail, _>(Postgres);
    let mut streamed = query
        .stream(pool, by_email("dan@example.com"))
        .expect("open the prepared stream");
    let mut streamed_ids = Vec::new();
    while let Some(id) = streamed.next().await {
        streamed_ids.push(id.expect("decode a streamed row"));
    }
    drop(streamed);
    assert_eq!(streamed_ids, vec![dan_id]);
}
