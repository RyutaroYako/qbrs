//! RIGHT JOIN / FULL JOIN: the table(s) already in scope retroactively
//! become nullable, mirroring Drizzle's `AppendToNullabilityMap` rule.
//! Run: `cargo run -p qbrs-examples --example 08_right_full_join`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // FROM orders, RIGHT JOIN users: `orders` was in scope first (from the
    // FROM clause) as not-null, but RIGHT JOIN flips every *already*-joined
    // table to nullable before adding the new one — so `orders::total`
    // decodes as `Option<i64>` here even though it was the FROM table,
    // purely because of how it ends up on the outer side of this RIGHT JOIN.
    let mut rows: Vec<(String, Option<i64>)> = select((users::email, orders::total))
        .from::<Postgres, _>(orders::Table)
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
    // Dan has no orders but must still appear (RIGHT JOIN keeps every user).
    assert!(rows.contains(&("dan@example.com".to_string(), None)));

    // FULL JOIN: both sides become nullable. To see it produce a genuinely
    // unmatched row we'd need an order with no matching user, which the
    // schema's FK constraint rules out — so here every row still has both
    // sides present, but the *type* (`Option<String>, Option<i64>`) is
    // honest about what FULL JOIN can produce in general, independent of
    // what today's data happens to contain.
    let mut full: Vec<(Option<String>, Option<i64>)> = select((users::email, orders::total))
        .from::<Postgres, _>(users::Table)
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
    assert!(full.contains(&(Some("dan@example.com".to_string()), None)));
}
