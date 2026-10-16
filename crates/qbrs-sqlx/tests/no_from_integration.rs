mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[tokio::test]
async fn a_select_with_no_from_takes_and_releases_an_advisory_lock_and_computes_a_row() {
    let (pool, guard) = common::test_pool("qbrs_no_from").await;

    // Running these two on `&pool` could release the lock from a connection
    // that never took it, since a session lock belongs to its connection.
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
