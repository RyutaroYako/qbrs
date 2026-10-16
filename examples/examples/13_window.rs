//! Window functions: `row_number()`/`rank()`/`dense_rank()`
//! `.over(window().partition_by(..).order_by(..))`. These return a
//! `WindowFunc<K, S>`, not an `Expr`. Its only method is `.over(..)`, so it
//! can't be used as an ordinary expression without one, and the result
//! carries the function itself as its row key, so `row.row_number()` reads
//! it back with nothing declared.
//! Known limitation: only niladic ranking functions so far.
//! `sum(col) OVER (..)` needs the real function-call design already
//! deferred for `count()`.
//!
//! `rank()` leaves a gap after ties where `dense_rank()` does not. The seed
//! data has no ties, so the two agree here.
//! Run: `cargo run -p qbrs-examples --example 13_window`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let ranked_within_user = select((
        users::email,
        orders::total,
        row_number().over(
            window()
                .partition_by(users::id)
                .order_by(orders::total.desc()),
        ),
    ))
    .from(users::Table)
    .inner_join(orders::Table, orders::user_id.eq(users::id))
    .order_by(users::email.asc())
    .load(&pool)
    .await
    .expect("ranked orders");

    println!("orders ranked within each user (email, total, row_number):");
    for row in &ranked_within_user {
        println!("  {} {} {}", row.email(), row.total(), row.row_number());
    }

    // Not left unlabelled: two `row_number()`s share one row key, so
    // `.get()` on either would be ambiguous.
    label!(within_user, overall);

    let ranked = select((
        users::email,
        orders::total,
        row_number()
            .over(
                window()
                    .partition_by(users::id)
                    .order_by(orders::total.desc()),
            )
            .label(label::within_user),
        row_number()
            .over(window().order_by(orders::total.desc()))
            .label(label::overall),
    ))
    .from(users::Table)
    .inner_join(orders::Table, orders::user_id.eq(users::id))
    .load(&pool)
    .await
    .expect("two rankings");

    println!("\nboth rankings side by side (email, total, within_user, overall):");
    for row in &ranked {
        println!(
            "  {} {} {} {}",
            row.email(),
            row.total(),
            row.within_user(),
            row.overall(),
        );
    }

    let overall = select((
        users::email,
        orders::total,
        rank().over(window().order_by(orders::total.desc())),
        dense_rank().over(window().order_by(orders::total.desc())),
    ))
    .from(users::Table)
    .inner_join(orders::Table, orders::user_id.eq(users::id))
    .load(&pool)
    .await
    .expect("overall rank");
    println!("\noverall order rank across all users:");
    for row in &overall {
        println!(
            "  {} {} rank={} dense_rank={}",
            row.email(),
            row.total(),
            row.rank(),
            row.dense_rank()
        );
    }
}
