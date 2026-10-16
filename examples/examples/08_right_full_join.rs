//! RIGHT JOIN / FULL JOIN: the table(s) already in scope retroactively
//! become nullable, mirroring Drizzle's `AppendToNullabilityMap` rule.
//!
//! `orders` enters scope not-null as the FROM table, and the RIGHT JOIN flips
//! it to nullable before adding `users`. So `orders::total` decodes as
//! `Option<i64>` even though it was the FROM table. A FULL JOIN makes both
//! sides nullable. The foreign key rules out an order with no user, so only
//! the users side produces unmatched rows here. The type
//! `(Option<String>, Option<i64>)` still states what a FULL JOIN can produce.
//! Run: `cargo run -p qbrs-examples --example 08_right_full_join`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let mut rows: Vec<(String, Option<i64>)> = select((users::email, orders::total))
        .from(orders::Table)
        .right_join(users::Table, orders::user_id.eq(users::id))
        .load(&pool)
        .await
        .expect("right join select")
        .into_tuples();
    rows.sort();

    println!("orders RIGHT JOIN users (email, total):");
    for row in &rows {
        println!("  {row:?}");
    }
    assert!(
        rows.contains(&("dan@example.com".to_string(), None)),
        "RIGHT JOIN keeps every user, Dan with no orders included"
    );

    let mut full: Vec<(Option<String>, Option<i64>)> = select((users::email, orders::total))
        .from(users::Table)
        .full_join(orders::Table, orders::user_id.eq(users::id))
        .load(&pool)
        .await
        .expect("full join select")
        .into_tuples();
    full.sort();

    println!("\nusers FULL JOIN orders (email, total):");
    for row in &full {
        println!("  {row:?}");
    }
    assert!(
        full.contains(&(Some("dan@example.com".to_string()), None)),
        "FULL JOIN keeps the user with no orders"
    );
}
