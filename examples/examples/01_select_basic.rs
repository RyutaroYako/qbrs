//! The simplest end-to-end query, and what a tuple selection decodes to: a
//! `Row` read by the same column values that selected it, not by position.
//! Run: `cargo run -p qbrs-examples --example 01_select_basic`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let active_users = select((users::id, users::email, users::display_name))
        .from(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::id.asc())
        .limit(10)
        .load(&pool)
        .await
        .expect("select active users");

    println!("Active users:");
    for row in &active_users {
        // Two ways to read the same field. `.get(users::email)` takes the
        // exact value that appeared in the selection list; `.email()` is the
        // accessor `#[derive(Table)]` generated for that column.
        println!(
            "  id={} email={} display_name={:?}",
            row.get(users::id),
            row.email(),
            row.display_name(),
        );
    }
    assert_eq!(
        active_users.len(),
        2,
        "seed() creates 3 users, 1 deactivated"
    );

    // Adding a column to the selection above would leave every read below
    // untouched — nothing here depends on a column's position. Where
    // destructuring is what's wanted, `into_tuples()` gives the positional
    // view back.
    let as_tuples: Vec<(i64, String, Option<String>)> = active_users.into_tuples();
    for (id, email, display_name) in &as_tuples {
        println!("  ({id}, {email:?}, {display_name:?})");
    }

    // A single un-tupled column decodes to its bare value, with no row to
    // index into.
    let emails: Vec<String> = select(users::email)
        .from(users::Table)
        .load(&pool)
        .await
        .expect("select emails");
    println!("all emails: {emails:?}");
}
