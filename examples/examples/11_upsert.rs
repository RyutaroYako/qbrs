//! Upsert: `ON CONFLICT (..) DO NOTHING` / `DO UPDATE SET ..`, gated at
//! compile time to dialects with `SupportsOnConflict` (Postgres, SQLite —
//! not MySQL, whose `ON DUPLICATE KEY UPDATE` is a different shape and a
//! separate future API). `ConflictTarget` proves the target column(s)
//! actually belong to the table being inserted into, and `partial_index(..)`
//! repeats an index's own predicate so a *partial* unique index can be the
//! one inferred.
//! Known limitation: no typed way yet to reference `EXCLUDED.column`.
//! Run: `cargo run -p qbrs-examples --example 11_upsert`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;

    let (id, display_name): (i64, Option<String>) = insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("grace@example.com")
                .display_name("Grace Hopper")
                .build(),
        )
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
    insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("grace@example.com")
                .display_name("Someone Else")
                .build(),
        )
        .on_conflict_do_nothing(users::email)
        .execute(&pool)
        .await
        .expect("upsert do-nothing");

    let unchanged: Option<String> = select(users::display_name)
        .from(users::Table)
        .filter(users::id.eq(id))
        .load_one(&pool)
        .await
        .expect("query after do-nothing")
        .expect("row still exists");
    assert_eq!(unchanged, display_name);
    println!("after DO NOTHING, display_name is still: {unchanged:?}");

    // Same conflicting email, but this time DO UPDATE SET reuses the same
    // `*Update` struct an `UPDATE` assigns from — only the fields actually set on
    // it are updated.
    let updated: Option<String> = insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("grace@example.com")
                .display_name("Someone Else")
                .build(),
        )
        .on_conflict_do_update(
            users::email,
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("Grace Brewster Hopper".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
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

    // A conflict target of bare columns is inferred against an index over
    // exactly those columns whose predicate the target's implies — and no
    // predicate implies nothing, so a *partial* unique index needs its own
    // repeated. `orders` has no unique constraint on `user_id`, so the
    // partial index below is the only one there is to infer.
    sqlx::query(
        "CREATE UNIQUE INDEX orders_one_open_per_user ON orders (user_id) WHERE NOT shipped",
    )
    .execute(&pool)
    .await
    .expect("create the partial unique index");
    insert(orders::Table)
        .values(OrdersInsert::builder().user_id(id).total(100).build())
        .execute(&pool)
        .await
        .expect("the user's open order");

    let raised: i64 = insert(orders::Table)
        .values(OrdersInsert::builder().user_id(id).total(250).build())
        .on_conflict_do_update(
            partial_index(orders::user_id, orders::shipped.eq(false)),
            Assignments::from_row(OrdersUpdate {
                total: Some(250),
                ..Default::default()
            })
            .expect("total is set"),
        )
        .returning(orders::total)
        .load_one(&pool)
        .await
        .expect("upsert against the partial unique index")
        .expect("returning row");
    println!("the open order's total is now: {raised}");
    assert_eq!(raised, 250);

    // Without the predicate there is no index over `user_id` alone to
    // infer, and the database says so rather than picking another.
    let unqualified = insert(orders::Table)
        .values(OrdersInsert::builder().user_id(id).total(999).build())
        .on_conflict_do_nothing(orders::user_id)
        .execute(&pool)
        .await;
    println!("without the index predicate: {}", unqualified.unwrap_err());
}
