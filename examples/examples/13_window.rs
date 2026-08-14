//! Window functions: `row_number()`/`rank()`/`dense_rank()`
//! `.over(window().partition_by(..).order_by(..))`. These return a
//! `WindowFunc<S>`, not an `Expr<Req, S>` — its only method is `.over(..)`,
//! so it can't be used as an ordinary expression without one.
//! Known limitation: only niladic ranking functions so far —
//! `sum(col) OVER (..)` needs the real function-call design already
//! deferred for `count()`.
//! Run: `cargo run -p qbrs-examples --example 13_window`

use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::select::{OrderExt, select};
use qbrs::window::{rank, row_number, window};
use qbrs_examples::{orders, seed, setup_db, users};
use qbrs_sqlx::LoadExt;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // Rank each user's own orders by size, largest first — `PARTITION BY`
    // restarts the numbering for every user, exactly like the SQL itself.
    let rows: Vec<(String, i64, i64)> = select((
        users::email,
        orders::total,
        row_number().over(
            window()
                .partition_by(users::id)
                .order_by(orders::total.desc()),
        ),
    ))
    .from::<Postgres, _>(users::Table)
    .inner_join(orders::Table, orders::user_id.eq(users::id))
    .order_by(users::email.asc())
    .load(&pool)
    .await
    .expect("ranked orders");

    println!("orders ranked within each user (email, total, rank):");
    for (email, total, row_num) in &rows {
        println!("  ({email:?}, {total}, {row_num})");
    }

    // `rank()` leaves a gap after ties (unlike `row_number()`, which never
    // ties) — not demonstrated with real ties here since the seed data has
    // none, but the same query shape applies.
    let overall: Vec<(String, i64, i64)> = select((
        users::email,
        orders::total,
        rank().over(window().order_by(orders::total.desc())),
    ))
    .from::<Postgres, _>(users::Table)
    .inner_join(orders::Table, orders::user_id.eq(users::id))
    .load(&pool)
    .await
    .expect("overall rank");
    println!("overall order rank across all users: {overall:?}");
}
