//! `INSERT INTO t (..) SELECT ..`: the archival copy — rows a query
//! produces rather than rows the caller holds. The target's own columns
//! are the header, and `row::SameShape` checks the query against them at
//! compile time, the same one comparison a `UNION` branch goes through.
//! Known limitation: no `ON CONFLICT` on this shape, and no column subset.
//! Run: `cargo run -p qbrs-examples --example 25_insert_select`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

/// The archive's columns are the source's, and its key is a plain column
/// so a copied one is accepted — `GENERATED ALWAYS` would refuse it.
#[derive(Table)]
#[table(name = "archived_orders")]
#[allow(dead_code)]
struct ArchivedOrders {
    #[column(primary_key)]
    id: i64,
    user_id: i64,
    total: i64,
    shipped: bool,
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    sqlx::query("DROP TABLE IF EXISTS archived_orders")
        .execute(&pool)
        .await
        .expect("drop archived_orders");
    sqlx::query(
        "CREATE TABLE archived_orders (
            id BIGINT PRIMARY KEY,
            user_id BIGINT NOT NULL,
            total BIGINT NOT NULL,
            shipped BOOLEAN NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("create archived_orders");

    // The source is an ordinary query — filters, joins, whatever it takes
    // to say which rows to copy. Its bind is numbered by the statement it
    // lands in, not by the query on its own.
    let shipped = select(orders::All)
        .from(orders::Table)
        .filter(orders::shipped.eq(true));

    let archived: Vec<i64> = insert(archived_orders::Table)
        .select(&shipped)
        .returning(archived_orders::id)
        .load(&pool)
        .await
        .expect("copy the shipped orders into the archive");
    println!("archived order ids: {archived:?}");
    assert_eq!(archived.len(), 1);

    // Then the originals go, which is the other half of the pattern.
    let removed = qbrs::delete::delete(orders::Table)
        .filter(orders::shipped.eq(true))
        .execute(&pool)
        .await
        .expect("delete the orders that were archived");
    println!("removed from orders: {removed}");
    assert_eq!(removed, 1);

    let left: i64 = select(qbrs::expr::count())
        .from(orders::Table)
        .load_one(&pool)
        .await
        .expect("count what is left")
        .expect("one row");
    println!("orders left: {left}");
    assert_eq!(left, 2);
}
