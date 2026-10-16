//! `UNION`/`UNION ALL`/`INTERSECT`/`EXCEPT` between two `SELECT`s with
//! different scopes: only their *output shape* has to match, not their
//! `Scope`. Ordering the combined result uses ordinal position
//! (`ORDER BY 1`), the only reference SQL itself allows once branches with
//! different scopes have been combined. `order_by_column` names the column,
//! and the row's own index for it supplies the position.
//! Run: `cargo run -p qbrs-examples --example 12_union`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    label!(total);

    // Not a bare `sql!(BigInt, "0")`: a literal has no name of its own, and
    // the branches must agree on names because the result is read by key.
    let user_rows = select((users::email, sql!(BigInt, "0").label(label::total)))
        .from(users::Table)
        .filter(users::active.eq(true));
    let order_rows = select((users::email, orders::total))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id));

    let feed: Vec<(String, i64)> = user_rows
        .union_all(&order_rows)
        .order_by_column(users::email, SortDir::Asc)
        .load(&pool)
        .await
        .expect("union_all feed")
        .into_tuples();
    println!("activity feed (email, amount):");
    for (email, amount) in &feed {
        println!("  ({email:?}, {amount})");
    }

    let active_emails = select((users::email,))
        .from(users::Table)
        .filter(users::active.eq(true));
    let ordering_emails = select((users::email,))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id));

    let active_and_ordered: Vec<(String,)> = active_emails
        .intersect(&ordering_emails)
        .order_by_column(users::email, SortDir::Asc)
        .load(&pool)
        .await
        .expect("intersect")
        .into_tuples();
    println!("active users who also ordered: {active_and_ordered:?}");

    let active_never_ordered: Vec<(String,)> = active_emails
        .except(&ordering_emails)
        .order_by_column(users::email, SortDir::Asc)
        .load(&pool)
        .await
        .expect("except")
        .into_tuples();
    println!("active users who never ordered: {active_never_ordered:?}");
}
