//! Upsert: `ON CONFLICT (..) DO NOTHING` / `DO UPDATE SET ..`, gated at
//! compile time to dialects with `SupportsOnConflict`. That is Postgres and
//! SQLite, not MySQL, whose `ON DUPLICATE KEY UPDATE` is a different shape
//! and a separate future API. `ConflictTarget` proves the target column(s)
//! actually belong to the table being inserted into, and `partial_index(..)`
//! repeats an index's own predicate so a *partial* unique index can be the
//! one inferred. `excluded(..)` names the row the insert proposed, which is
//! what an accumulating upsert reads and what a value passed to both halves
//! of the statement cannot say. `.filter(..)` on that list decides whether
//! the update fires at all: a rejected row is not counted, so a one-row
//! upsert's `execute` answers "did this write anything".
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

    // `email` already exists, so DO NOTHING means this row is silently
    // skipped and the original `display_name` survives untouched.
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
    // `*Update` struct an `UPDATE` assigns from. Only the fields actually
    // set on it are updated.
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
    // exactly those columns whose predicate the target's implies. No
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

    // Passing the same Rust value to both halves of an upsert stands in
    // for the proposed row only while the value is one the caller holds.
    // `excluded(..)` names that row itself, so the assignment can read it
    // and the conflicting row together. Here that adds to a running total
    // rather than replacing it.
    let accumulated: i64 = insert(orders::Table)
        .values(OrdersInsert::builder().user_id(id).total(75).build())
        .on_conflict_do_update(
            partial_index(orders::user_id, orders::shipped.eq(false)),
            ConflictUpdate::set_to(
                orders::total,
                qbrs::sql!(
                    qbrs::expr::BigInt,
                    "(? + ?)",
                    orders::total,
                    excluded(orders::total)
                ),
            ),
        )
        .returning(orders::total)
        .load_one(&pool)
        .await
        .expect("accumulate onto the open order")
        .expect("returning row");
    println!("after adding the proposed row's total: {accumulated}");
    assert_eq!(accumulated, 325);

    // A `WHERE` on the `DO UPDATE` itself: the row is touched only if the
    // condition holds, and a rejected one is not counted. That is what
    // lets a one-row upsert's count answer "was this already done?". An
    // unconditional `DO UPDATE` always reports 1.
    let raise_once = |total: i64| {
        insert(orders::Table)
            .values(OrdersInsert::builder().user_id(id).total(total).build())
            .on_conflict_do_update(
                partial_index(orders::user_id, orders::shipped.eq(false)),
                ConflictUpdate::set_to(orders::total, excluded(orders::total))
                    .filter(orders::total.lt(excluded(orders::total))),
            )
    };
    let raised_again = raise_once(500)
        .execute(&pool)
        .await
        .expect("a higher total");
    let refused = raise_once(10).execute(&pool).await.expect("a lower total");
    println!("raising the total wrote {raised_again} row(s); lowering it wrote {refused}");
    assert_eq!((raised_again, refused), (1, 0));

    // Without the predicate there is no index over `user_id` alone to
    // infer, and the database says so rather than picking another.
    let unqualified = insert(orders::Table)
        .values(OrdersInsert::builder().user_id(id).total(999).build())
        .on_conflict_do_nothing(orders::user_id)
        .execute(&pool)
        .await;
    println!("without the index predicate: {}", unqualified.unwrap_err());
}
