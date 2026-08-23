//! INSERT: `Defaultable<T>` (omit vs. explicit value vs. explicit NULL),
//! bulk insert, and `RETURNING`.
//! Run: `cargo run -p qbrs-examples --example 03_insert`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;

    // `email` is required (NOT NULL, no default), so `build()` is out of
    // reach until it's given. `display_name` is nullable with no default,
    // so it's omitted (stays NULL) unless `.display_name(..)` is called.
    // `active` is NOT NULL with a schema default, so omitting it renders
    // the SQL keyword `DEFAULT` rather than sending a value at all.
    let (id, email, display_name): (i64, String, Option<String>) = insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("grace@example.com")
                .display_name("Grace Hopper")
                .build(),
        )
        .returning((users::id, users::email, users::display_name))
        .load(&pool)
        .await
        .expect("insert one user")
        .into_iter()
        .next()
        .expect("returning row")
        .into_tuple();
    println!("inserted: id={id} email={email} display_name={display_name:?}");

    // Bulk insert in one statement, mixing rows that do/don't override
    // `display_name` — each row independently uses DEFAULT/a bound value
    // for the columns it omits/sets, all sharing the same column list.
    let ids: Vec<i64> = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .values(
            UsersInsert::builder()
                .email("b@example.com")
                .display_name("B")
                .build(),
        )
        .returning(users::id)
        .load(&pool)
        .await
        .expect("bulk insert");
    println!("bulk inserted ids: {ids:?}");
}
