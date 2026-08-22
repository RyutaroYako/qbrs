//! INSERT: `Defaultable<T>` (omit vs. explicit value vs. explicit NULL),
//! bulk insert, and `RETURNING`.
//! Run: `cargo run -p qbrs-examples --example 03_insert`

use qbrs::dialect::Postgres;
use qbrs_examples::{UsersInsert, setup_db, users};
use qbrs_sqlx::LoadReturningExt;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;

    // `email` is required (NOT NULL, no default) — `new()` only asks for
    // that. `display_name` is nullable with no default, so it's omitted
    // (stays NULL) unless `.display_name(..)` is called. `active` is
    // NOT NULL with a schema default, so omitting it renders the SQL
    // keyword `DEFAULT` rather than sending a value at all.
    let (id, email, display_name): (i64, String, Option<String>) =
        qbrs::insert::insert::<Postgres, _>(users::Table)
            .values(UsersInsert::new("grace@example.com").display_name("Grace Hopper"))
            .returning((users::id, users::email, users::display_name))
            .load(&pool)
            .await
            .expect("insert one user")
            .into_iter()
            .next()
            .expect("returning row")
            .into();
    println!("inserted: id={id} email={email} display_name={display_name:?}");

    // Bulk insert in one statement, mixing rows that do/don't override
    // `display_name` — each row independently uses DEFAULT/a bound value
    // for the columns it omits/sets, all sharing the same column list.
    let ids: Vec<i64> = qbrs::insert::insert::<Postgres, _>(users::Table)
        .values(UsersInsert::new("a@example.com"))
        .values(UsersInsert::new("b@example.com").display_name("B"))
        .returning(users::id)
        .load(&pool)
        .await
        .expect("bulk insert");
    println!("bulk inserted ids: {ids:?}");
}
