//! The `sql!{}` escape hatch: SQL the builder doesn't cover yet, still
//! type-tagged, still parameterized (never string-spliced), and still
//! scope-checked where a slot holds a column.
//!
//! A `?` slot takes a column as readily as a value. The renderer writes the
//! column out (quoted, table-qualified) and binds the value as a parameter, so
//! only the text between slots is unchecked. Because the slot carries its
//! table, `orders::total` in a slot without `orders` joined is a compile
//! error, not a database error.
//! Run: `cargo run -p qbrs-examples --example 06_raw_sql`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let matches: Vec<String> = select(users::email)
        .from(users::Table)
        .filter(sql!(Bool, "lower(?) LIKE ?", users::email, "%@example.com"))
        .filter(users::active.eq(true))
        .load(&pool)
        .await
        .expect("raw sql filter");

    println!("emails matching raw LIKE filter: {matches:?}");
    assert_eq!(matches.len(), 2);

    label!(biggest);
    let per_user: Vec<(String, Option<i64>)> = select((
        users::email,
        // Not `BigInt`: `max` over no rows is NULL, and the type written in
        // `sql!` is the type it decodes to.
        sql!(Nullable<BigInt>, "max(?)", orders::total).label(label::biggest),
    ))
    .from(users::Table)
    .left_join(orders::Table, orders::user_id.eq(users::id))
    .group_by(users::email)
    .load(&pool)
    .await
    .expect("raw aggregate")
    .into_tuples();

    println!("largest order per user: {per_user:?}");
    assert!(
        per_user.contains(&("dan@example.com".to_string(), None)),
        "Dan has no orders, so his max is NULL"
    );
}
