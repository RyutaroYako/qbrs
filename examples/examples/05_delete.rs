//! DELETE, with `RETURNING` to see what was removed.
//! Run: `cargo run -p qbrs-examples --example 05_delete`

use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs_examples::{seed, setup_db, users};
use qbrs_sqlx::LoadReturningExt;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // A single bare `Column` (not wrapped in a tuple) decodes to its plain
    // native type, not a 1-tuple — `.returning((users::email,))` would give
    // `Vec<(String,)>` instead.
    let deleted: Vec<String> = qbrs::delete::delete::<Postgres, _>(users::Table)
        .filter(users::active.eq(false))
        .returning(users::email)
        .load(&pool)
        .await
        .expect("delete inactive users");

    println!("deleted inactive users: {deleted:?}");
    assert_eq!(deleted, vec!["dan@example.com".to_string()]);
}
