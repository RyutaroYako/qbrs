//! Self-join workaround: qbrs has no table aliasing (see README's Known
//! limitations), so a literal `FROM "orders" AS a JOIN "orders" AS b` can't
//! be written. Binding a `with!{}` pseudo-table to a plain `SELECT` over the
//! *same* table gets the same result — each order paired with its sibling
//! orders from the same user — fully type-checked, with no new API beyond
//! `with!{}` + `cte::with(..)`, already used for real CTEs in `14_cte`.
//! Run: `cargo run -p qbrs-examples --example 21_self_join_via_cte`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

with! {
    struct sibling_orders { id: BigInt, user_id: BigInt, total: BigInt }
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let all_orders = select((orders::id, orders::user_id, orders::total)).from(orders::Table);

    let pairs: Vec<(i64, i64)> = select((orders::id, sibling_orders::id))
        .from(orders::Table)
        .inner_join(
            cte::with(sibling_orders::Table, &all_orders),
            sibling_orders::user_id.eq(orders::user_id),
        )
        .filter(sibling_orders::id.ne(orders::id))
        .order_by(orders::id.asc())
        .load(&pool)
        .await
        .expect("sibling order pairs")
        .into_tuples();

    println!("orders paired with a sibling order from the same user: {pairs:?}");
    // Ada has two orders, so each is the other's sibling; Dan has none and
    // Grace has exactly one, so neither appears on either side.
    assert_eq!(pairs.len(), 2);
}
