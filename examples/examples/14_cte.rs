//! Common table expressions: `with!{}` declares a named pseudo-table,
//! `cte::with()` binds a query to it (checked against the declared column
//! shape at compile time), and the binding goes wherever a table goes
//! (`.from(..)`, `.inner_join(..)`), after which the CTE's table behaves
//! exactly like a real one. Columns are always rendered with an explicit
//! name list (`WITH name (col, ..) AS (..)`), since a computed expression
//! like `sum(..)` has no column name of its own. `sum(col)` is still named
//! after its column, which satisfies the CTE's declared `total` with nothing
//! labelled, and a missing join in the body is a compile error as anywhere.
//! Known limitation: non-recursive, single-level CTEs only. `WITH RECURSIVE`
//! and chaining CTEs are deferred.
//! Run: `cargo run -p qbrs-examples --example 14_cte`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

with! {
    struct big_spenders {
        user_id: BigInt,
        // Not `BigInt`: `sum(..)` is typed `Nullable<BigInt>`, since it is NULL over
        // zero rows, and the body must match this row.
        total: Nullable<BigInt>,
    }
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let spend_over_1000_per_user = select((orders::user_id, sum(orders::total)))
        .from(orders::Table)
        .group_by(orders::user_id)
        .having(sum(orders::total).gt(1000i64));

    let rows = select((users::email, big_spenders::total))
        .from(users::Table)
        .inner_join(
            cte::with(big_spenders::Table, &spend_over_1000_per_user),
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
