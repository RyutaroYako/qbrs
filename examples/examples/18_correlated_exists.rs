//! Correlated subqueries: `EXISTS (SELECT .. WHERE inner.col = outer.col)`.
//! The subquery is built *from* the outer query, so its scope is the outer
//! one plus its own table, which is what makes the outer column reference
//! legal, and what tags the resulting condition with the tables it needs, so
//! it can only be filtered back onto a query that has them.
//!
//! `.correlated(..)` borrows the outer query, so `users_query` survives to be
//! filtered with the condition built from it. A subquery is a full builder,
//! so `GROUP BY` and `HAVING` answer "users with more than one order".
//! Run: `cargo run -p qbrs-examples --example 18_correlated_exists`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let users_query = select((users::email,))
        .from(users::Table)
        .order_by(users::id.asc());

    let has_order = users_query
        .correlated(orders::Table, (orders::id,))
        .filter(orders::user_id.eq(users::id));

    let with_orders: Vec<String> = users_query
        .clone()
        .filter(has_order.exists())
        .load(&pool)
        .await
        .expect("users with an order")
        .into_tuples()
        .into_iter()
        .map(|(email,)| email)
        .collect();
    println!("users with at least one order: {with_orders:?}");

    let without: Vec<(String,)> = users_query
        .clone()
        .filter(has_order.not_exists())
        .load(&pool)
        .await
        .expect("users with no order")
        .into_tuples();
    println!("users with none: {without:?}");
    assert!(
        without.contains(&("dan@example.com".to_string(),)),
        "Dan has no orders"
    );

    // Not `(orders::id,)` as above: `EXISTS` ignores what a subquery selects,
    // but the database still checks the selection against `GROUP BY`.
    let more_than_one = users_query
        .correlated(orders::Table, (orders::user_id,))
        .filter(orders::user_id.eq(users::id))
        .group_by(orders::user_id)
        .having(count().gt(1i64));

    let repeat: Vec<(String,)> = users_query
        .filter(more_than_one.exists())
        .load(&pool)
        .await
        .expect("repeat customers")
        .into_tuples();
    println!("users with more than one order: {repeat:?}");
    assert!(
        repeat.contains(&("ada@example.com".to_string(),)),
        "Ada has two orders"
    );
}
