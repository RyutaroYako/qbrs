//! A `SELECT` with no `FROM`: a value the database computes rather than a
//! row it reads, such as `now()`, `current_setting('..')`, or an advisory
//! lock. The seed goes straight to a terminal, and what makes that safe is
//! the empty scope: a column has nowhere to resolve, so naming one does not
//! compile, as `tests/compile-bench/trybuild-drafts/column_without_from.rs.draft`
//! shows. An advisory lock serialises a job across app instances with no
//! table involved, just a function of a bound key.
//! Run: `cargo run -p qbrs-examples --example 26_no_from`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;

    // Not `&pool`: a session lock belongs to the connection that took it, and
    // the pool could hand the unlock a different one.
    let mut conn = pool.acquire().await.expect("one connection");
    let job_key = 90_210_i64;

    let taken: bool = select(sql!(Bool, "pg_try_advisory_lock(?)", job_key))
        .load_one(&mut *conn)
        .await
        .expect("try the advisory lock")
        .expect("one row");
    println!("took the lock: {taken}");
    assert!(taken);

    if taken {
        let released: bool = select(sql!(Bool, "pg_advisory_unlock(?)", job_key))
            .load_one(&mut *conn)
            .await
            .expect("release the advisory lock")
            .expect("one row");
        println!("released: {released}");
        assert!(released);
    }
    drop(conn);

    let (zone, sum): (String, i64) = select((
        sql!(Text, "current_setting('TimeZone')"),
        sql!(BigInt, "(? + ?)", 20i64, 22i64),
    ))
    .load_one(&pool)
    .await
    .expect("read a setting and compute a sum")
    .expect("one row")
    .into_tuple();
    println!("server time zone {zone}, and 20 + 22 = {sum}");
    assert_eq!(sum, 42);
}
