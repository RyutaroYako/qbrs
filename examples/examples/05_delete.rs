//! DELETE, with `RETURNING` to see what was removed.
//! Run: `cargo run -p qbrs-examples --example 05_delete`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // A single bare `Column` (not wrapped in a tuple) decodes to its plain
    // native type, not a 1-tuple — `.returning((users::email,))` would give
    // `Vec<(String,)>` instead.
    let deleted: Vec<String> = delete(users::Table)
        .filter(users::active.eq(false))
        .returning(users::email)
        .load(&pool)
        .await
        .expect("delete inactive users");

    println!("deleted inactive users: {deleted:?}");
    assert_eq!(deleted, vec!["dan@example.com".to_string()]);
}
