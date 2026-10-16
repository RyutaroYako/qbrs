//! DELETE, with `RETURNING` to see what was removed. A single bare column in
//! `RETURNING` decodes to its native type, so the result is a `Vec<String>`
//! rather than a list of rows.
//! Run: `cargo run -p qbrs-examples --example 05_delete`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let deleted: Vec<String> = delete(users::Table)
        .filter(users::active.eq(false))
        .returning(users::email)
        .load(&pool)
        .await
        .expect("delete inactive users");

    println!("deleted inactive users: {deleted:?}");
    assert_eq!(deleted, vec!["dan@example.com".to_string()]);
}
