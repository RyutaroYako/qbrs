//! `prepare!{}`: a query rendered once, reused across many `.execute(params)`
//! calls with different, compile-time-typed values — closing the gap
//! Drizzle's own `sql.placeholder()` leaves (its `.execute()` takes an
//! untyped `Record<string, unknown>`, so a missing/misspelled key is only
//! a runtime error; here it's the exact struct `prepare!{}` generated).
//! Run: `cargo run -p qbrs-examples --example 10_prepared`

use qbrs::dialect::Postgres;
use qbrs::expr::{ExprMethods, Text};
use qbrs::prepare;
use qbrs::select::select;
use qbrs_examples::{seed, setup_db, users};
use qbrs_sqlx::PreparedExt;

prepare! {
    struct ByEmail { email: Text }
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // Rendered once. `ByEmail::email()` is a placeholder, not a value yet.
    let query = select((users::id, users::display_name))
        .from::<Postgres, _>(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare::<ByEmail, _>();

    for email in [
        "ada@example.com",
        "grace@example.com",
        "no-such-user@example.com",
    ] {
        let rows: Vec<(i64, Option<String>)> = query
            .execute(
                &pool,
                ByEmail {
                    email: email.to_string(),
                },
            )
            .await
            .expect("execute prepared query");
        println!("{email} -> {rows:?}");
    }
}
