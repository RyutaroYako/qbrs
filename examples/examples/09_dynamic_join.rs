//! `DynSelect`: the one case the type system genuinely cannot express —
//! conditionally joining a table or not — handled via a narrow, explicit
//! erasure hatch instead of a Drizzle-style `.$dynamic()` on the whole
//! query. Run: `cargo run -p qbrs-examples --example 09_dynamic_join`

use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::select::{DynSelect, select};
use qbrs_examples::{orders, seed, setup_db, users};
use qbrs_sqlx::LoadDynExt;

/// A single static return type cannot mean "joined orders" in one branch
/// and "didn't" in another — `.erase()` unifies them into `DynSelect`.
fn build_query(include_orders: bool) -> DynSelect<Postgres, String> {
    let base = select(users::email).from::<Postgres, _>(users::Table);
    if include_orders {
        base.inner_join(orders::Table, orders::user_id.eq(users::id))
            .erase()
    } else {
        base.erase()
    }
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let without_join: Vec<String> = build_query(false).load(&pool).await.expect("without join");
    println!("without join: {without_join:?}");
    assert_eq!(without_join.len(), 3); // all users, incl. Dan (no orders)

    let with_join: Vec<String> = build_query(true).load(&pool).await.expect("with join");
    println!("with join: {with_join:?}");
    assert_eq!(with_join.len(), 3); // ada x2 orders + grace x1, Dan drops out (INNER JOIN)
}
