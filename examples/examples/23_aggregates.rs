//! Every aggregate helper, and the two questions `count` answers.
//!
//! An aggregate is keyed by the column it aggregates, so a row of them is read
//! by the value that selected it, like any other row. `count()` counts rows
//! and `count_of(column)` counts that column's non-NULL values. They agree
//! until a LEFT JOIN makes them differ.
//! Run: `cargo run -p qbrs-examples --example 23_aggregates`

use qbrs::expr::{avg, count, count_of, max, min, string_agg, sum};
use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let per_user = select((
        users::email,
        count(),
        count_of(orders::total),
        sum(orders::total),
        min(orders::total),
        max(orders::total),
        avg(orders::total),
    ))
    .from(users::Table)
    .left_join(orders::Table, orders::user_id.eq(users::id))
    .group_by(users::email)
    .having(count_of(orders::total).gt(0i64))
    .order_by(users::email.asc())
    .load(&pool)
    .await
    .expect("aggregate each user's orders");

    println!("Per user, over the orders they have:");
    for row in &per_user {
        println!(
            "  {:<20} rows={} orders={} sum={:?} min={:?} max={:?} avg={:?}",
            row.email(),
            row.get(count()),
            row.get(count_of(orders::total)),
            row.get(sum(orders::total)),
            row.get(min(orders::total)),
            row.get(max(orders::total)),
            row.get(avg(orders::total)),
        );
    }
    assert_eq!(per_user.len(), 2, "dan has no orders, so HAVING drops him");

    let dan_rows_and_orders: Vec<(i64, i64)> = select((count(), count_of(orders::total)))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::email.eq("dan@example.com"))
        .load(&pool)
        .await
        .expect("count rows against count of a column")
        .into_tuples();
    assert_eq!(
        dan_rows_and_orders,
        vec![(1, 0)],
        "Dan's LEFT JOIN yields one row holding no order"
    );

    // Not a runtime `String`: MySQL's `SEPARATOR` takes only a literal, so the
    // separator is a `&'static str` on every dialect, though Postgres binds it.
    let emails: Option<String> = select(string_agg(users::email, ", "))
        .from(users::Table)
        .filter(users::active.eq(true))
        .load_one(&pool)
        .await
        .expect("concatenate the active users' emails")
        .expect("one row");
    println!("active: {}", emails.as_deref().unwrap_or("(none)"));

    // Not the builder's `string_agg`: ordering inside an aggregate is not
    // built, since SQLite has it only from 3.44 and MySQL spells it elsewhere.
    let ordered: Option<String> = select(qbrs::sql!(
        Nullable<Text>,
        "string_agg(?, ', ' ORDER BY ?)",
        users::email,
        users::email
    ))
    .from(users::Table)
    .load_one(&pool)
    .await
    .expect("an ordered string_agg through the escape hatch")
    .expect("one row");
    println!("all, in order: {}", ordered.as_deref().unwrap_or("(none)"));
}
