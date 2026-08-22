//! Common table expressions: `with!{}` declares a named pseudo-table,
//! `cte::with()` binds a query to it (checked against the declared column
//! shape at compile time), and the binding goes wherever a table goes —
//! `.from(..)`, `.inner_join(..)` — after which the CTE's table behaves
//! exactly like a real one. Columns are
//! always rendered with an explicit name list (`WITH name (col, ..) AS (..)`),
//! since a computed expression like `sum(..)` has no column name of its own.
//! Known limitation: non-recursive, single-level CTEs only — `WITH RECURSIVE`
//! and chaining CTEs are deferred.
//! Run: `cargo run -p qbrs-examples --example 14_cte`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

with! {
    struct big_spenders { user_id: BigInt, total: Nullable<BigInt> }
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // The CTE body: total spend per user, restricted to users who've spent
    // over 1000. `sum(col)` is a real aggregate over a real column, so a
    // missing join is a compile error here just as it is anywhere else, and
    // the result is named after its column — which is what satisfies the
    // CTE's declared `total` with nothing labelled. It decodes as
    // `Option<i64>`: a sum over zero rows is NULL, whatever the column says.
    let totals = select((orders::user_id, sum(orders::total)))
        .from::<Postgres, _>(orders::Table)
        .group_by(orders::user_id)
        .having(sum(orders::total).gt(1000i64));

    let rows = select((users::email, big_spenders::total))
        .from::<Postgres, _>(users::Table)
        .inner_join(
            cte::with(big_spenders::Table, &totals),
            big_spenders::user_id.eq(users::id),
        )
        .load(&pool)
        .await
        .expect("big spenders");

    println!("users who've spent over 1000 (email, total):");
    for row in &rows {
        println!("  ({:?}, {:?})", row.email(), row.get(big_spenders::total));
    }
}
