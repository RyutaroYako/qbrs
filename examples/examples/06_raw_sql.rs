//! The `sql!{}` escape hatch: SQL the builder doesn't cover yet, still
//! type-tagged and still parameterized (never string-spliced).
//! Run: `cargo run -p qbrs-examples --example 06_raw_sql`

use qbrs::dialect::Postgres;
use qbrs::expr::{Bool, ExprMethods};
use qbrs::select::select;
use qbrs::sql;
use qbrs_examples::{seed, setup_db, users};
use qbrs_sqlx::LoadExt;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // `sql!` builds an `Expr<Nil, Bool>` — a raw fragment exempt from
    // scope-checking (the caller is trusted to reference real, in-scope
    // columns by name), but its `?` placeholder is still bound as a real
    // parameter, not spliced into the SQL text.
    let matches: Vec<String> = select(users::email)
        .from::<Postgres, _>(users::Table)
        .filter(sql!(Bool, "lower(email) LIKE ?", "%@example.com"))
        .filter(users::active.eq(true))
        .load(&pool)
        .await
        .expect("raw sql filter");

    println!("emails matching raw LIKE filter: {matches:?}");
    assert_eq!(matches.len(), 2);
}
