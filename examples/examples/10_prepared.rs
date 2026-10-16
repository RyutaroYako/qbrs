//! `prepare!{}`: a query rendered once, reused across many `.load(.., params)`
//! calls with different, compile-time-typed values. That closes the gap
//! Drizzle's own `sql.placeholder()` leaves: its `.execute()` takes an
//! untyped `Record<string, unknown>`, so a missing/misspelled key is only
//! a runtime error. Here it is the exact struct `prepare!{}` generated.
//! `ByEmail::email()` is a placeholder, not a value, until a call binds it. A
//! count can be prepared from the same filter, rendered once beside the rows.
//! Run: `cargo run -p qbrs-examples --example 10_prepared`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

prepare! {
    struct ByEmail { email: Text }
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let users_by_email = select((users::id, users::display_name))
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare::<ByEmail, _>(Postgres);

    let count_by_email = select((users::id,))
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare_count::<ByEmail, _>(Postgres);

    for email in [
        "ada@example.com",
        "grace@example.com",
        "no-such-user@example.com",
    ] {
        let rows: Vec<(i64, Option<String>)> = users_by_email
            .load(
                &pool,
                ByEmail {
                    email: email.to_string(),
                },
            )
            .await
            .expect("load prepared query")
            .into_tuples();
        let matching = count_by_email
            .count(
                &pool,
                ByEmail {
                    email: email.to_string(),
                },
            )
            .await
            .expect("count prepared query");
        println!("{email} -> {rows:?} ({matching} row(s))");
    }
}
