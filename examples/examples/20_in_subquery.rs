//! `x IN (<subquery>)` / `x NOT IN (<subquery>)`: `Select::contains` builds
//! the condition from a subquery the same way `.exists()` does, but the
//! subquery here selects exactly one column, checked against the outer
//! expression with the same `Comparable` rule `.eq(..)` uses.
//! Run: `cargo run -p qbrs-examples --example 20_in_subquery`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // A single un-tupled selection, exactly like `select(users::email)`
    // elsewhere. That's what lets `.contains(..)` read its `RowField::Sql`
    // and check it against `lhs` the way `.eq(..)` checks two columns.
    let big_spenders = select(orders::user_id)
        .from(orders::Table)
        .filter(orders::total.gt(1000i64));

    let spenders: Vec<String> = select((users::email,))
        .from(users::Table)
        .filter(big_spenders.contains(users::id))
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("users who spent over 1000")
        .into_tuples()
        .into_iter()
        .map(|(email,)| email)
        .collect();
    println!("spent over 1000: {spenders:?}");
    assert_eq!(spenders, vec!["ada@example.com".to_string()]);

    // `big_spenders` is borrowed by `.contains(..)`, not consumed, so the
    // same subquery serves both sides of the split.
    let everyone_else: Vec<String> = select((users::email,))
        .from(users::Table)
        .filter(big_spenders.not_contains(users::id))
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("users who didn't spend over 1000")
        .into_tuples()
        .into_iter()
        .map(|(email,)| email)
        .collect();
    println!("everyone else: {everyone_else:?}");
    assert_eq!(
        everyone_else,
        vec![
            "dan@example.com".to_string(),
            "grace@example.com".to_string()
        ]
    );
}
