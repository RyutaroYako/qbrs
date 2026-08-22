//! Transactions against a real Postgres: `LoadExt`/`ExecuteExt`/etc. are
//! generic over `sqlx::PgExecutor`, so a `sqlx::PgTransaction` works
//! everywhere a `&PgPool` does — see `postgres_integration.rs` for the
//! DB-setup rationale (Docker vs. `postgresql_embedded`). Kept as a single
//! test function (rather than one per scenario) so every scenario shares
//! one `tx_users` table without racing against parallel test threads.

mod common;

use qbrs::Table;
use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::row::IntoTuples;
use qbrs::select::select;
use qbrs_sqlx::{ExecuteExt, LoadExt};

#[derive(Table)]
#[table(name = "tx_users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
}

#[tokio::test]
async fn transactions_against_real_postgres() {
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

    // Committed transaction: writes made through `&mut *tx` persist once
    // `.commit()` succeeds.
    let mut tx = pool.begin().await.expect("begin transaction");
    let ada_id: i64 = qbrs::insert::insert::<Postgres, _>(users::Table)
        .values(UsersInsert::builder().email("ada@example.com").build())
        .returning(users::id)
        .load(&mut *tx)
        .await
        .expect("insert inside transaction")
        .into_iter()
        .next()
        .expect("returning row");
    tx.commit().await.expect("commit transaction");

    let row: Option<String> = select(users::email)
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(ada_id))
        .load_one(&pool)
        .await
        .expect("select after commit");
    assert_eq!(row, Some("ada@example.com".to_string()));

    // Explicit rollback: the insert is visible inside the transaction but
    // never lands once rolled back.
    let mut tx = pool.begin().await.expect("begin transaction");
    let dan_id: i64 = qbrs::insert::insert::<Postgres, _>(users::Table)
        .values(UsersInsert::builder().email("dan@example.com").build())
        .returning(users::id)
        .load(&mut *tx)
        .await
        .expect("insert inside transaction")
        .into_iter()
        .next()
        .expect("returning row");
    let visible_in_tx: Option<String> = select(users::email)
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(dan_id))
        .load_one(&mut *tx)
        .await
        .expect("select inside transaction");
    assert_eq!(visible_in_tx, Some("dan@example.com".to_string()));
    tx.rollback().await.expect("rollback transaction");

    let row: Option<String> = select(users::email)
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(dan_id))
        .load_one(&pool)
        .await
        .expect("select after rollback");
    assert_eq!(row, None);

    // Dropping a transaction without `.commit()` rolls it back implicitly,
    // same as plain sqlx.
    let grace_id = {
        let mut tx = pool.begin().await.expect("begin transaction");
        let id: i64 = qbrs::insert::insert::<Postgres, _>(users::Table)
            .values(UsersInsert::builder().email("grace@example.com").build())
            .returning(users::id)
            .load(&mut *tx)
            .await
            .expect("insert inside transaction")
            .into_iter()
            .next()
            .expect("returning row");
        id
    };
    let row: Option<String> = select(users::email)
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(grace_id))
        .load_one(&pool)
        .await
        .expect("select after implicit rollback");
    assert_eq!(row, None);

    // Insert + update + delete inside one transaction, committed together.
    let mut tx = pool.begin().await.expect("begin transaction");
    let ids: Vec<i64> = qbrs::insert::insert::<Postgres, _>(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .values(UsersInsert::builder().email("b@example.com").build())
        .returning(users::id)
        .load(&mut *tx)
        .await
        .expect("bulk insert inside transaction");
    assert_eq!(ids.len(), 2);

    let affected = qbrs::update::update::<Postgres, _>(users::Table)
        .set(UsersUpdate {
            email: Some("a2@example.com".to_string()),
        })
        .expect("email is set")
        .filter(users::id.eq(ids[0]))
        .execute(&mut *tx)
        .await
        .expect("update inside transaction");
    assert_eq!(affected, 1);

    let deleted = qbrs::delete::delete::<Postgres, _>(users::Table)
        .filter(users::id.eq(ids[1]))
        .execute(&mut *tx)
        .await
        .expect("delete inside transaction");
    assert_eq!(deleted, 1);
    tx.commit().await.expect("commit transaction");

    let mut remaining: Vec<(i64, String)> = select((users::id, users::email))
        .from::<Postgres, _>(users::Table)
        .load(&pool)
        .await
        .expect("select remaining users")
        .into_tuples();
    remaining.sort();
    assert_eq!(
        remaining,
        vec![
            (ada_id, "ada@example.com".to_string()),
            (ids[0], "a2@example.com".to_string())
        ]
    );

    common::shutdown(pool, guard).await;
}
