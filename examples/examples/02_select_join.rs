//! LEFT JOIN with automatically-derived NULL-ability: `orders::total`
//! decodes as `Option<i64>` with no manual `.nullable()` annotation, purely
//! because the join is a LEFT JOIN. This is the crate's central
//! differentiator over diesel, which requires that annotation manually.
//!
//! Dan has no orders, so under the LEFT JOIN his `total` comes back `None`:
//! the join-derived `Option<i64>` is exercised at runtime, not only claimed.
//! Under an INNER JOIN he disappears instead, and `orders::total` decodes as
//! plain `i64`. The type itself documents that this query never sees a NULL.
//! Run: `cargo run -p qbrs-examples --example 02_select_join`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let rows = select((users::email, orders::total))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("left join select");

    println!("All users LEFT JOIN orders (email, total):");
    for row in &rows {
        let left_join_total_is_optional: &Option<i64> = row.total();
        println!("  {} {:?}", row.email(), left_join_total_is_optional);
    }
    let mut flat: Vec<(String, Option<i64>)> = rows.into_tuples();
    flat.sort();
    assert!(
        flat.contains(&("dan@example.com".to_string(), None)),
        "Dan has no orders, so the LEFT JOIN gives him a NULL total"
    );

    let inner = select((users::email, orders::total))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(orders::total.desc())
        .load(&pool)
        .await
        .expect("inner join select");

    println!("\nusers INNER JOIN orders (email, total), highest first:");
    for row in &inner {
        let inner_join_total_is_never_null: &i64 = row.total();
        println!("  {} {}", row.email(), inner_join_total_is_never_null);
    }
    assert!(
        !inner.iter().any(|row| row.email() == "dan@example.com"),
        "Dan has no orders, so the INNER JOIN drops him"
    );
}
