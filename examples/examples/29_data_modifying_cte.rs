//! A data-modifying CTE: `WITH shipped AS (UPDATE .. RETURNING ..) SELECT ..`.
//! Postgres alone takes a write statement as a `WITH` body, so it is gated
//! on `SupportsDataModifyingCte`. That is what turns "write the row, then
//! read a value the row doesn't hold" into one round-trip instead of two
//! statements pinned to the same transaction.
//! The join belongs to the outer query, so it can be a `LEFT JOIN`, which
//! is what a `RETURNING` naming a joined column would not have given.
//! Known limitations: every part of such a statement sees one snapshot, so
//! the outer query reads the CTE's own returned rows rather than the table
//! it wrote, and the second-to-last query shows what that means; the query
//! binding a write body has to *be* the statement, which the compiler does
//! not check, and the last query is the one Postgres refuses.
//! Run: `cargo run -p qbrs-examples --example 29_data_modifying_cte`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

with! {
    struct shipped { id: BigInt, user_id: BigInt, total: BigInt }
}

with! {
    struct touched { id: BigInt, email: Text }
}

/// The DTO a handler would return: the written row, plus the owner's email,
/// which the order row holds only as a `user_id`.
#[derive(qbrs::FromRow, Debug)]
#[allow(dead_code)]
struct Shipped {
    #[from_row(from = shipped::total)]
    total: i64,
    #[from_row(from = users::email)]
    email: String,
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // One statement: mark every unshipped order shipped, and hand back each
    // one with the email of the user who placed it.
    let ship = update(orders::Table)
        .set_to(orders::shipped, true)
        .filter(orders::shipped.eq(false))
        .returning((orders::id, orders::user_id, orders::total));

    let notified: Vec<Shipped> = select((shipped::total, users::email))
        .from(cte::with(shipped::Table, &ship))
        .inner_join(users::Table, users::id.eq(shipped::user_id))
        .order_by(shipped::total.asc())
        .load(&pool)
        .await
        .expect("ship the orders and read their owners")
        .into_structs();
    println!("shipped, with the owner each belongs to: {notified:?}");
    assert!(!notified.is_empty());

    // The reason this replaces a `RETURNING` that names a joined column
    // rather than merely standing in for one: the join belongs to the outer
    // query, so it can be a `LEFT JOIN`. A user with no orders is written
    // and still comes back. That is the row a second query handled by not
    // running.
    let deactivate = update(users::Table)
        .set_to(users::active, false)
        .returning((users::id, users::email));
    let touched_rows: Vec<(String, Option<i64>)> = select((touched::email, orders::total))
        .from(cte::with(touched::Table, &deactivate))
        .left_join(orders::Table, orders::user_id.eq(touched::id))
        .order_by(touched::email.asc())
        .load(&pool)
        .await
        .expect("deactivate everyone and read whatever orders each has")
        .into_tuples();
    println!("deactivated, with each one's orders: {touched_rows:?}");
    assert!(touched_rows.iter().any(|(_, total)| total.is_none()));

    // The write is real, not just returned.
    let unshipped: i64 = select(qbrs::expr::count())
        .from(orders::Table)
        .filter(orders::shipped.eq(false))
        .load_one(&pool)
        .await
        .expect("count what is left")
        .expect("one row");
    println!("orders still unshipped: {unshipped}");
    assert_eq!(unshipped, 0);

    // One snapshot for the whole statement: an outer query that joins the
    // table the body *wrote* sees it as it was before. That is why the CTE
    // returns its own rows: they are the only view of the new values.
    let stale = update(orders::Table)
        .set_to(orders::total, 1i64)
        .returning((orders::id, orders::user_id, orders::total));
    let as_it_was: Vec<i64> = select(orders::total)
        .from(cte::with(shipped::Table, &stale))
        .inner_join(orders::Table, orders::id.eq(shipped::id))
        .order_by(orders::total.asc())
        .load(&pool)
        .await
        .expect("read the written table from inside the same statement");
    println!("the same statement still sees the old totals: {as_it_was:?}");
    assert!(!as_it_was.contains(&1));

    // A write body has to be the top-level statement's. Nesting the query
    // that binds one (here as an `EXISTS` subquery) type-checks and is
    // refused by the server, which is the limitation the module documents.
    let nested = update(orders::Table)
        .set_to(orders::total, 2i64)
        .returning((orders::id, orders::user_id, orders::total));
    let inner = select((shipped::id,)).from(cte::with(shipped::Table, &nested));
    let refused = select(users::id)
        .from(users::Table)
        .filter(inner.exists())
        .load(&pool)
        .await;
    println!("a write CTE below the top level: {}", refused.unwrap_err());
}
