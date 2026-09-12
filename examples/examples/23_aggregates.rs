//! Every aggregate helper, and the two questions `count` answers.
//! Run: `cargo run -p qbrs-examples --example 23_aggregates`

use qbrs::expr::{avg, count, count_of, max, min, string_agg, sum};
use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // An aggregate is keyed by the column it aggregates, so a row of six of
    // them is read the way any other row is — by the value that selected it.
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

    // `count()` counts rows and `count_of(column)` counts that column's
    // non-NULL values — the same number until a LEFT JOIN makes them differ.
    // Dan matches no order, so his row has one row and no order in it.
    let dan: Vec<(i64, i64)> = select((count(), count_of(orders::total)))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::email.eq("dan@example.com"))
        .load(&pool)
        .await
        .expect("count rows against count of a column")
        .into_tuples();
    assert_eq!(dan, vec![(1, 0)]);

    // `string_agg` takes its separator as a `&'static str`, written into the
    // SQL rather than bound: MySQL's `SEPARATOR` takes a literal and rejects
    // a parameter, so a separator that renders in only two of the three
    // dialects is not one this builder offers. It is NULL over zero rows,
    // hence the `Option`.
    let emails: Option<String> = select(string_agg(users::email, ", "))
        .from(users::Table)
        .filter(users::active.eq(true))
        .load_one(&pool)
        .await
        .expect("concatenate the active users' emails")
        .expect("one row");
    println!("active: {}", emails.as_deref().unwrap_or("(none)"));

    // Ordering *inside* the aggregate isn't built — SQLite reached it only
    // in 3.44 and MySQL spells it elsewhere in the call — so a run whose
    // order matters goes through `sql!{}`, where the text is yours.
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
