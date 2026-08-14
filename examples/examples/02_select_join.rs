//! LEFT JOIN with automatically-derived NULL-ability: `orders::total`
//! decodes as `Option<i64>` with no manual `.nullable()` annotation, purely
//! because the join is a LEFT JOIN — this is the crate's central
//! differentiator over diesel (which requires that annotation manually).
//! Run: `cargo run -p qbrs-examples --example 02_select_join`

use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::select::{OrderExt, select};
use qbrs_examples::{orders, seed, setup_db, users};
use qbrs_sqlx::LoadExt;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // Dan has zero orders (seed() gives him none) — his row's `total` must
    // come back `None`, proving the join-derived Option<i64> is real, not
    // just a type-level claim that never gets exercised at runtime.
    let mut rows: Vec<(String, Option<i64>)> = select((users::email, orders::total))
        .from::<Postgres, _>(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("left join select");
    rows.sort();

    println!("All users LEFT JOIN orders (email, total):");
    for row in &rows {
        println!("  {row:?}");
    }
    assert!(rows.contains(&("dan@example.com".to_string(), None)));

    // INNER JOIN instead: Dan (no orders) disappears entirely rather than
    // appearing with a NULL total. Note `orders::total` here decodes as
    // plain `i64`, not `Option<i64>` — INNER JOIN doesn't introduce
    // nullability, so `Selection` doesn't wrap it, and the *type itself*
    // documents that this query can never see a NULL total.
    let inner: Vec<(String, i64)> = select((users::email, orders::total))
        .from::<Postgres, _>(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(orders::total.desc())
        .load(&pool)
        .await
        .expect("inner join select");

    println!("\nusers INNER JOIN orders (email, total), highest first:");
    for row in &inner {
        println!("  {row:?}");
    }
    assert!(!inner.iter().any(|(email, _)| email == "dan@example.com"));
}
