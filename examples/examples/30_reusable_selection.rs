//! Naming a selection once and using it in several places. A tuple of
//! columns is a value, so a `const` names it; and a tuple is one element of
//! a longer list, the way `<table>::All` is, so the same name goes into a
//! bare `select(..)` and into a wider one beside an aggregate. No macro
//! expands the list at each call site, so appending an aggregate needs no
//! second arm. Reading a row cannot tell how the list was assembled: the keys
//! are the columns', so `.get(..)` takes the column either way.
//! Run: `cargo run -p qbrs-examples --example 30_reusable_selection`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

type OrderCore = (
    Column<orders::columns::id>,
    Column<orders::columns::user_id>,
    Column<orders::columns::total>,
);
const ORDER_CORE: OrderCore = (orders::id, orders::user_id, orders::total);

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

    let list_endpoint: Vec<(i64, i64, i64)> = select(ORDER_CORE)
        .from(orders::Table)
        .order_by(orders::id.asc())
        .load(&pool)
        .await
        .expect("list the orders")
        .into_tuples();
    println!("orders: {list_endpoint:?}");
    assert_eq!(list_endpoint.len(), 3);

    let detail_endpoint: Vec<OrderWithOwner> = select((ORDER_CORE, users::email))
        .from(orders::Table)
        .inner_join(users::Table, users::id.eq(orders::user_id))
        .order_by(orders::id.asc())
        .load(&pool)
        .await
        .expect("list the orders with their owner")
        .into_structs();
    println!("with owners: {detail_endpoint:?}");
    assert_eq!(detail_endpoint.len(), 3);

    // Not grouped by all three columns: grouping by the primary key lets the
    // rest of the reused columns come along.
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
