mod common;

use qbrs::Table;
use qbrs::expr::ExprMethods;
use qbrs::row::IntoTuples;
use qbrs::select::select;
use qbrs::statement::Statement;
use qbrs::update::Assignments;
use qbrs_sqlx::{ExecuteExt, LoadExt};
use sqlx::PgPool;

#[derive(Table)]
#[table(name = "tx_users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
}

async fn stored_email(pool: &PgPool, id: i64) -> Option<String> {
    select(users::email)
        .from(users::Table)
        .filter(users::id.eq(id))
        .load_one(pool)
        .await
        .expect("select by id")
}

// Splitting these scenarios into one test each would race them on the one
// `tx_users` table, since tests in a binary run in parallel.
#[tokio::test]
async fn every_builder_runs_inside_a_transaction_through_mut_tx_as_it_does_on_the_pool() {
    let (pool, guard) = common::test_pool("qbrs_test_tx").await;

    sqlx::query("DROP TABLE IF EXISTS tx_users")
        .execute(&pool)
        .await
        .expect("drop table");
    sqlx::query(
        "CREATE TABLE tx_users (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            email TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("create table");

    let ada_id = an_insert_through_a_transaction_persists_once_it_commits(&pool).await;
    a_rolled_back_insert_is_visible_inside_its_transaction_and_absent_after(&pool).await;
    dropping_a_transaction_without_committing_rolls_its_insert_back(&pool).await;
    an_insert_update_and_delete_in_one_transaction_commit_together(&pool, ada_id).await;

    common::shutdown(pool, guard).await;
}

async fn an_insert_through_a_transaction_persists_once_it_commits(pool: &PgPool) -> i64 {
    let mut tx = pool.begin().await.expect("begin transaction");
    let ada_id: i64 = qbrs::insert::insert(users::Table)
        .values(UsersInsert::builder().email("ada@example.com").build())
        .returning(users::id)
        .load_one(&mut *tx)
        .await
        .expect("insert inside transaction")
        .expect("returning row");
    tx.commit().await.expect("commit transaction");

    assert_eq!(
        stored_email(pool, ada_id).await,
        Some("ada@example.com".to_string())
    );
    ada_id
}

async fn a_rolled_back_insert_is_visible_inside_its_transaction_and_absent_after(pool: &PgPool) {
    let mut tx = pool.begin().await.expect("begin transaction");
    let dan_id: i64 = qbrs::insert::insert(users::Table)
        .values(UsersInsert::builder().email("dan@example.com").build())
        .returning(users::id)
        .load_one(&mut *tx)
        .await
        .expect("insert inside transaction")
        .expect("returning row");
    let visible_in_tx: Option<String> = select(users::email)
        .from(users::Table)
        .filter(users::id.eq(dan_id))
        .load_one(&mut *tx)
        .await
        .expect("select inside transaction");
    assert_eq!(visible_in_tx, Some("dan@example.com".to_string()));
    tx.rollback().await.expect("rollback transaction");

    assert_eq!(stored_email(pool, dan_id).await, None);
}

async fn dropping_a_transaction_without_committing_rolls_its_insert_back(pool: &PgPool) {
    let grace_id = {
        let mut tx = pool.begin().await.expect("begin transaction");
        qbrs::insert::insert(users::Table)
            .values(UsersInsert::builder().email("grace@example.com").build())
            .returning(users::id)
            .load_one(&mut *tx)
            .await
            .expect("insert inside transaction")
            .expect("returning row")
    };
    assert_eq!(stored_email(pool, grace_id).await, None);
}

async fn an_insert_update_and_delete_in_one_transaction_commit_together(
    pool: &PgPool,
    ada_id: i64,
) {
    let mut tx = pool.begin().await.expect("begin transaction");
    let ids: Vec<i64> = qbrs::insert::insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .values(UsersInsert::builder().email("b@example.com").build())
        .returning(users::id)
        .load(&mut *tx)
        .await
        .expect("bulk insert inside transaction");
    let [renamed_id, deleted_id] = ids[..] else {
        panic!("expected two ids, got {ids:?}");
    };

    let affected = qbrs::update::update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                email: Some("a2@example.com".to_string()),
            })
            .expect("email is set"),
        )
        .filter(users::id.eq(renamed_id))
        .execute(&mut *tx)
        .await
        .expect("update inside transaction");
    assert_eq!(affected, 1);

    let deleted = qbrs::delete::delete(users::Table)
        .filter(users::id.eq(deleted_id))
        .execute(&mut *tx)
        .await
        .expect("delete inside transaction");
    assert_eq!(deleted, 1);
    tx.commit().await.expect("commit transaction");

    let mut remaining: Vec<(i64, String)> = select((users::id, users::email))
        .from(users::Table)
        .load(pool)
        .await
        .expect("select remaining users")
        .into_tuples();
    remaining.sort();
    assert_eq!(
        remaining,
        vec![
            (ada_id, "ada@example.com".to_string()),
            (renamed_id, "a2@example.com".to_string())
        ]
    );
}
