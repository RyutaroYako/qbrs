//! `SELECT <expr>` with no `FROM`, against a real Postgres: the advisory
//! lock the issue reached raw sqlx for, plus the two other shapes a
//! FROM-less query has — a function of no arguments, and one taking a bind.

mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[tokio::test]
async fn a_select_with_no_from_computes_its_value_and_binds_its_arguments() {
    let (pool, guard) = common::test_pool("qbrs_no_from").await;

    // The shape the issue was stuck on: a lock keyed by a bound value,
    // with no table anywhere in the statement. A session lock belongs to
    // the connection that took it, so both statements go to the same one —
    // which a FROM-less query reaches the way any other does, by being
    // generic over `PgExecutor`.
    let mut conn = pool.acquire().await.expect("one connection");
    let key = 4_242_i64;
    let taken: bool = select(qbrs::sql!(qbrs::expr::Bool, "pg_try_advisory_lock(?)", key))
        .load_one(&mut *conn)
        .await
        .expect("take the advisory lock")
        .expect("one row");
    assert!(taken);

    let released: bool = select(qbrs::sql!(qbrs::expr::Bool, "pg_advisory_unlock(?)", key))
        .load_one(&mut *conn)
        .await
        .expect("release the advisory lock")
        .expect("one row");
    assert!(released);
    drop(conn);

    // A function of no arguments, and a row of two values rather than one.
    let settings: (String, i64) = select((
        qbrs::sql!(qbrs::expr::Text, "current_setting('server_version')"),
        qbrs::sql!(qbrs::expr::BigInt, "(? + ?)", 2i64, 3i64),
    ))
    .load_one(&pool)
    .await
    .expect("read a setting and compute a sum")
    .expect("one row")
    .into_tuple();
    assert!(!settings.0.is_empty());
    assert_eq!(settings.1, 5);

    common::shutdown(pool, guard).await;
}
