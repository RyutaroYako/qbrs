//! `prepare!{}`: a query rendered once, reused across many `.load(.., params)`
//! calls with different, compile-time-typed values — closing the gap
//! Drizzle's own `sql.placeholder()` leaves (its `.execute()` takes an
//! untyped `Record<string, unknown>`, so a missing/misspelled key is only
//! a runtime error; here it's the exact struct `prepare!{}` generated).
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

    // Rendered once. `ByEmail::email()` is a placeholder, not a value yet.
    let query = select((users::id, users::display_name))
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare::<ByEmail, _>();

    // The same query prepared as its own total: one rendering for the page,
    // one for the count, and the page size bound rather than baked in.
    let total = select((users::id,))
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare_count::<ByEmail, _>();

    for email in [
        "ada@example.com",
        "grace@example.com",
        "no-such-user@example.com",
    ] {
        let rows: Vec<(i64, Option<String>)> = query
            .load(
                &pool,
                ByEmail {
                    email: email.to_string(),
                },
            )
            .await
            .expect("load prepared query")
            .into_tuples();
        let matching = total
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
