//! `DynSelect`: the one place a query's *shape* has to be decided at
//! runtime. A single static type cannot mean "joined orders" in one branch
//! and "didn't" in another, so `.erase()` unifies them, dropping the join
//! skeleton and nothing else.
//! Run: `cargo run -p qbrs-examples --example 09_dynamic_join`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

/// Erasure keeps the row, so the return type has to name it. A row's type is
/// its key list, which is long by construction, so an alias says it
/// once. `clippy::type_complexity` counts the nesting, hence the allow.
#[allow(clippy::type_complexity)]
type UserRow =
    Row<RowCons<users::columns::id, i64, RowCons<users::columns::email, String, RowNil>>>;

fn page(include_orders: bool, page: u32) -> DynSelect<Postgres, UserRow> {
    let base = select((users::id, users::email))
        .from(users::Table)
        .order_by(users::id.asc());

    // `order_by` has to happen before `.erase()`: a sort key is a column
    // reference, and the scope that justifies it is what erasure gives up.
    // `limit`/`offset` reference nothing, so they survive it and the
    // pagination tail isn't duplicated across the branches.
    if include_orders {
        base.inner_join(orders::Table, orders::user_id.eq(users::id))
            .erase()
    } else {
        base.erase()
    }
    .limit(10)
    .offset(page * 10)
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    for include_orders in [false, true] {
        let rows = page(include_orders, 0).load(&pool).await.expect("page");
        println!("include_orders={include_orders}:");
        for row in &rows {
            println!("  {} {}", row.id(), row.email());
        }
    }

    // Both branches produce the same row, so anything that reads one reads
    // the other. That is the whole point of erasing only the join skeleton.
    let joined = page(true, 0).load(&pool).await.expect("joined");
    assert!(joined.iter().all(|row| *row.id() > 0));
}
