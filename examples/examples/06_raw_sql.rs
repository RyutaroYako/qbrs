//! The `sql!{}` escape hatch: SQL the builder doesn't cover yet, still
//! type-tagged, still parameterized (never string-spliced), and — where a
//! slot holds a column — still scope-checked.
//! Run: `cargo run -p qbrs-examples --example 06_raw_sql`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // A `?` slot takes a column as readily as a value: the column is
    // written out by the renderer — quoted, table-qualified — and the value
    // is bound as a parameter. Only `lower(..) LIKE ..` is unchecked text.
    let matches: Vec<String> = select(users::email)
        .from(users::Table)
        .filter(sql!(Bool, "lower(?) LIKE ?", users::email, "%@example.com"))
        .filter(users::active.eq(true))
        .load(&pool)
        .await
        .expect("raw sql filter");

    println!("emails matching raw LIKE filter: {matches:?}");
    assert_eq!(matches.len(), 2);

    // Because the slot carries its table, the fragment is checked against
    // the query's scope like anything else: putting `orders::total` in one
    // without joining `orders` is a compile error, not a database error. The
    // type written in the `sql!` is the type it decodes to — `max` over no
    // rows is NULL, so this one says so.
    label!(biggest);
    let per_user: Vec<(String, Option<i64>)> = select((
        users::email,
        sql!(Nullable<BigInt>, "max(?)", orders::total).label(label::biggest),
    ))
    .from(users::Table)
    .inner_join(orders::Table, orders::user_id.eq(users::id))
    .group_by(users::email)
    .load(&pool)
    .await
    .expect("raw aggregate")
    .into_tuples();

    println!("largest order per user: {per_user:?}");
}
