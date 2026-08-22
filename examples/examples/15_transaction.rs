//! Transactions: every `.load()`/`.execute()` method is generic over
//! `sqlx::PgExecutor`, so a `sqlx::PgTransaction` works everywhere a
//! `&PgPool` does — pass `&mut *tx`, exactly like plain sqlx usage.
//! Run: `cargo run -p qbrs-examples --example 15_transaction`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;

    // Commit: an order only exists once the user that owns it does, so both
    // inserts either land together or not at all.
    let mut tx = pool.begin().await.expect("begin transaction");

    let user_id: i64 = insert::<Postgres, _>(users::Table)
        .values(
            UsersInsert::builder()
                .email("nadia@example.com")
                .display_name("Nadia")
                .build(),
        )
        .returning(users::id)
        .load(&mut *tx)
        .await
        .expect("insert user")
        .into_iter()
        .next()
        .expect("returning row");

    insert::<Postgres, _>(orders::Table)
        .values(OrdersInsert::builder().user_id(user_id).total(4200).build())
        .execute(&mut *tx)
        .await
        .expect("insert order");

    tx.commit().await.expect("commit transaction");
    println!("committed: user {user_id} with one order");

    // Rollback: an error partway through leaves neither row behind — the
    // dropped `tx` (implicit rollback) or an explicit `.rollback()` both
    // undo everything since `.begin()`.
    let mut tx = pool.begin().await.expect("begin transaction");

    let doomed_id: i64 = insert::<Postgres, _>(users::Table)
        .values(UsersInsert::builder().email("temp@example.com").build())
        .returning(users::id)
        .load(&mut *tx)
        .await
        .expect("insert user")
        .into_iter()
        .next()
        .expect("returning row");

    tx.rollback().await.expect("rollback transaction");

    let still_there: Option<i64> = select(users::id)
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(doomed_id))
        .load_one(&pool)
        .await
        .expect("select rolled-back user");
    assert_eq!(still_there, None);
    println!("rolled back: user {doomed_id} was not persisted");
}
