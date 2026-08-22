//! UPDATE: only explicitly-`Some(..)` fields are sent, and nullable columns
//! distinguish "leave alone" from "set to NULL" via `Option<Option<T>>`.
//! Run: `cargo run -p qbrs-examples --example 04_update`

use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs_examples::{UsersUpdate, seed, setup_db, users};
use qbrs_sqlx::{ExecuteExt, LoadExt};

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let ada_id: i64 = qbrs::select::select(users::id)
        .from::<Postgres, _>(users::Table)
        .filter(users::email.eq("ada@example.com"))
        .load_one(&pool)
        .await
        .expect("query ada")
        .expect("ada exists");

    // Only `display_name` is sent in the SET clause — `email`/`active`
    // are entirely absent from the rendered SQL, not just left unchanged
    // via a redundant `email = email` self-assignment.
    let affected = qbrs::update::update::<Postgres, _>(users::Table)
        .set(UsersUpdate {
            display_name: Some(Some("Ada, Countess of Lovelace".into())),
            ..Default::default()
        })
        .expect("display_name is set")
        .filter(users::id.eq(ada_id))
        .execute(&pool)
        .await
        .expect("update display_name");
    println!("renamed {affected} row(s)");

    // `Some(None)` means "set this nullable column to NULL", distinct from
    // `None` ("don't touch it") — clearing the display name explicitly.
    let cleared = qbrs::update::update::<Postgres, _>(users::Table)
        .set(UsersUpdate {
            display_name: Some(None),
            ..Default::default()
        })
        .expect("display_name is set")
        .filter(users::id.eq(ada_id))
        .returning(users::display_name)
        .load(&pool)
        .await
        .expect("clear display_name");
    println!("after clearing: {cleared:?}");
    assert_eq!(cleared, vec![None]);
}
