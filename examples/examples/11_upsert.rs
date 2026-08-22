//! Upsert: `ON CONFLICT (..) DO NOTHING` / `DO UPDATE SET ..`, gated at
//! compile time to dialects with `SupportsOnConflict` (Postgres, SQLite —
//! not MySQL, whose `ON DUPLICATE KEY UPDATE` is a different shape and a
//! separate future API). `ConflictTarget` proves the target column(s)
//! actually belong to the table being inserted into.
//! Known limitation: no typed way yet to reference `EXCLUDED.column`.
//! Run: `cargo run -p qbrs-examples --example 11_upsert`

use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs_examples::{UsersInsert, UsersUpdate, setup_db, users};
use qbrs_sqlx::{ExecuteExt, LoadExt};

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;

    let (id, display_name): (i64, Option<String>) =
        qbrs::insert::insert::<Postgres, _>(users::Table)
            .values(UsersInsert::new("grace@example.com").display_name("Grace Hopper"))
            .returning((users::id, users::display_name))
            .load(&pool)
            .await
            .expect("insert grace")
            .into_iter()
            .next()
            .expect("returning row")
            .into_tuple();
    println!("inserted: id={id} display_name={display_name:?}");

    // `email` already exists — DO NOTHING means this row is silently
    // skipped, so the original `display_name` survives untouched.
    qbrs::insert::insert::<Postgres, _>(users::Table)
        .values(UsersInsert::new("grace@example.com").display_name("Someone Else"))
        .on_conflict_do_nothing(users::email)
        .execute(&pool)
        .await
        .expect("upsert do-nothing");

    let unchanged: Option<String> = qbrs::select::select(users::display_name)
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(id))
        .load_one(&pool)
        .await
        .expect("query after do-nothing")
        .expect("row still exists");
    assert_eq!(unchanged, display_name);
    println!("after DO NOTHING, display_name is still: {unchanged:?}");

    // Same conflicting email, but this time DO UPDATE SET reuses the same
    // `*Update` struct `.set(..)` takes — only the fields actually set on
    // it are updated.
    let updated: Option<String> = qbrs::insert::insert::<Postgres, _>(users::Table)
        .values(UsersInsert::new("grace@example.com").display_name("Someone Else"))
        .on_conflict_do_update(
            users::email,
            UsersUpdate {
                display_name: Some(Some("Grace Brewster Hopper".into())),
                ..Default::default()
            },
        )
        .returning(users::display_name)
        .load(&pool)
        .await
        .expect("upsert do-update")
        .into_iter()
        .next()
        .expect("returning row");
    println!("after DO UPDATE, display_name is now: {updated:?}");
    assert_eq!(updated, Some("Grace Brewster Hopper".to_string()));
}
