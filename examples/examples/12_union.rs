//! `UNION`/`UNION ALL`/`INTERSECT`/`EXCEPT` between two `SELECT`s that read
//! from different tables — only their *output shape* has to match, not
//! their `Scope`. Ordering the combined result uses ordinal position
//! (`ORDER BY 1`), the only reference SQL itself allows once branches with
//! different scopes have been combined.
//! Run: `cargo run -p qbrs-examples --example 12_union`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // Two structurally unrelated queries — one over `users`, one over
    // `orders` — unioned into a single "activity feed" of labeled emails
    // and amounts. There is no single `Scope` that contains both `users`
    // and `orders` here (no join at all), yet this still type-checks: the
    // only requirement is that both sides decode to the same
    // `(String, i64)` shape.
    // Branches must agree on column *names* as well as types, since the
    // combined result is read by key. A literal has no name of its own.
    label!(total);

    let user_rows = select((users::email, sql!(BigInt, "0").label(label::total)))
        .from(users::Table)
        .filter(users::active.eq(true));
    let order_rows = select((users::email, orders::total))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id));

    // Ordered by the column, not by counting to it — the position comes
    // from the row's own index for `users::email`.
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

    // `INTERSECT`: emails that appear both as an active user and as having
    // placed an order.
    let active_emails = select((users::email,))
        .from(users::Table)
        .filter(users::active.eq(true));
    let ordering_emails = select((users::email,))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id));

    let both: Vec<(String,)> = active_emails
        .intersect(&ordering_emails)
        .order_by(nth(1).asc())
        .load(&pool)
        .await
        .expect("intersect")
        .into_tuples();
    println!("active users who also ordered: {both:?}");
}
