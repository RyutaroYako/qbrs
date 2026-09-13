//! Naming a selection once and using it in several places. A tuple of
//! columns is a value, so a `const` names it; and a tuple is one element of
//! a longer list, the way `<table>::All` is, so the same name goes into a
//! bare `select(..)` and into a wider one beside an aggregate. No macro
//! expands the list at each call site.
//! Run: `cargo run -p qbrs-examples --example 30_reusable_selection`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

/// The columns a list endpoint and a detail endpoint share. Spelled once:
/// the type through each column's `columns::` marker, the value through the
/// column consts beside it.
type OrderCore = (
    Column<orders::columns::id>,
    Column<orders::columns::user_id>,
    Column<orders::columns::total>,
);
const ORDER_CORE: OrderCore = (orders::id, orders::user_id, orders::total);

/// What the wider query decodes to, filled by name like any other row.
#[derive(qbrs::FromRow, Debug)]
#[allow(dead_code)]
struct OrderWithOwner {
    total: i64,
    email: String,
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // The list endpoint: the selection on its own.
    let listing: Vec<(i64, i64, i64)> = select(ORDER_CORE)
        .from(orders::Table)
        .order_by(orders::id.asc())
        .load(&pool)
        .await
        .expect("list the orders")
        .into_tuples();
    println!("orders: {listing:?}");
    assert_eq!(listing.len(), 3);

    // The detail endpoint: the same name, one element of a longer list.
    let detailed: Vec<OrderWithOwner> = select((ORDER_CORE, users::email))
        .from(orders::Table)
        .inner_join(users::Table, users::id.eq(orders::user_id))
        .order_by(orders::id.asc())
        .load(&pool)
        .await
        .expect("list the orders with their owner")
        .into_structs();
    println!("with owners: {detailed:?}");
    assert_eq!(detailed.len(), 3);

    // And beside an aggregate, which is the case a hand-rolled macro needs a
    // second arm for. Grouping by the primary key is what lets the rest of
    // the reused columns come along.
    let counted: Vec<(i64, i64, i64, i64)> = select((ORDER_CORE, qbrs::expr::count()))
        .from(orders::Table)
        .group_by(orders::id)
        .order_by(orders::id.asc())
        .load(&pool)
        .await
        .expect("count beside the reused selection")
        .into_tuples();
    println!("with a count appended: {counted:?}");
    assert_eq!(counted.len(), 3);

    // Reading a row cannot tell how the list was assembled: the keys are the
    // columns', so `.get(..)` takes the column either way.
    let one = select((ORDER_CORE, users::email))
        .from(orders::Table)
        .inner_join(users::Table, users::id.eq(orders::user_id))
        .order_by(orders::id.asc())
        .load_one(&pool)
        .await
        .expect("read one row")
        .expect("a row");
    println!(
        "first order {} belongs to {}",
        one.get(orders::id),
        one.get(users::email)
    );
}
