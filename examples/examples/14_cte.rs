//! Common table expressions: `with!{}` declares a named pseudo-table,
//! `cte::with()` binds a query to it (checked against the declared column
//! shape at compile time), and `.with(cte)` attaches it to an outer query —
//! after that, the CTE's table behaves exactly like a real one. Columns are
//! always rendered with an explicit name list (`WITH name (col, ..) AS (..)`),
//! since a computed expression like `sum(..)` has no column name of its own.
//! Known limitation: non-recursive, single-level CTEs only — `WITH RECURSIVE`
//! and chaining CTEs are deferred.
//! Run: `cargo run -p qbrs-examples --example 14_cte`

use qbrs::dialect::Postgres;
use qbrs::expr::{BigInt, ExprMethods};
use qbrs::select::select;
use qbrs::sql;
use qbrs::with;
use qbrs_examples::{orders, seed, setup_db, users};
use qbrs_sqlx::LoadExt;

with! {
    struct big_spenders { user_id: qbrs::expr::BigInt, total: qbrs::expr::BigInt }
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // The CTE body: total spend per user, restricted to users who've spent
    // over 1000. `sum(..)` isn't a typed builtin yet (see `count()`'s doc
    // comment for why), so it's built via the `sql!{}` escape hatch here —
    // `big_spenders::Table` only exists once `cte::with(..)` binds a query
    // to it, and nothing else in the crate needs to know a CTE is involved,
    // since `big_spenders::Table` is a real `scope::Table` impl just like
    // `users::Table`.
    // Postgres's `sum(bigint)` returns `numeric`, not `bigint` — cast back
    // explicitly so it decodes as a plain `i64` on the Rust side.
    let totals = select((orders::user_id, sql!(BigInt, "sum(orders.total)::bigint")))
        .from::<Postgres, _>(orders::Table)
        .group_by(orders::user_id)
        .having(sql!(BigInt, "sum(orders.total)::bigint").gt(1000i64));

    let rows: Vec<(String, i64)> = select((users::email, big_spenders::total))
        .with(qbrs::cte::with(big_spenders::Table, &totals))
        .from::<Postgres, _>(users::Table)
        .inner_join(big_spenders::Table, big_spenders::user_id.eq(users::id))
        .load(&pool)
        .await
        .expect("big spenders");

    println!("users who've spent over 1000 (email, total):");
    for (email, total) in &rows {
        println!("  ({email:?}, {total})");
    }
}
