//! A `SELECT` with no `FROM`: a value the database computes rather than a
//! row it reads — `now()`, `current_setting('..')`, an advisory lock. The
//! seed goes straight to a terminal, and what makes that safe is the empty
//! scope: a column has nowhere to resolve, so naming one does not compile.
//! Run: `cargo run -p qbrs-examples --example 26_no_from`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;

    // Serialising a job across app instances: no table is involved, just a
    // function of a bound key. A session lock belongs to the connection
    // that took it, so the run holds one for both statements.
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
        // ... the work this lock is serialising ...
        let released: bool = select(sql!(Bool, "pg_advisory_unlock(?)", job_key))
            .load_one(&mut *conn)
            .await
            .expect("release the advisory lock")
            .expect("one row");
        println!("released: {released}");
        assert!(released);
    }
    drop(conn);

    // A row of several computed values reads like any other row.
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

    // A column would have nowhere to resolve, so this does not compile:
    //
    //     select((users::id,)).load(&pool)
    //     // error: `users::Table` is not available in this query's scope
}
