//! Window functions: `row_number()`/`rank()`/`dense_rank()`
//! `.over(window().partition_by(..).order_by(..))`. These return a
//! `WindowFunc<K, S>`, not an `Expr` — its only method is `.over(..)`, so it
//! can't be used as an ordinary expression without one — and the result
//! carries the function itself as its row key, so `row.row_number()` reads
//! it back with nothing declared.
//! Known limitation: only niladic ranking functions so far —
//! `sum(col) OVER (..)` needs the real function-call design already
//! deferred for `count()`.
//! Run: `cargo run -p qbrs-examples --example 13_window`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // Rank each user's own orders by size, largest first — `PARTITION BY`
    // restarts the numbering for every user, exactly like the SQL itself.
    let rows = select((
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
    for row in &rows {
        println!("  {} {} {}", row.email(), row.total(), row.row_number());
    }

    // Two `row_number()`s in one query would both want the same row key, so
    // `.get()` on either would be ambiguous. `label!` declares names for
    // them; the declared name reaches the SQL as the column's `AS` too.
    // Declaring it here rather than at module level keeps it next to the
    // query, and puts it out of reach of any local binding.
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

    // `rank()` leaves a gap after ties (unlike `row_number()`, which never
    // ties) — not demonstrated with real ties here since the seed data has
    // none, but the same query shape applies.
    let overall = select((
        users::email,
        orders::total,
        rank().over(window().order_by(orders::total.desc())),
    ))
    .from(users::Table)
    .inner_join(orders::Table, orders::user_id.eq(users::id))
    .load(&pool)
    .await
    .expect("overall rank");
    println!("\noverall order rank across all users:");
    for row in &overall {
        println!("  {} {} {}", row.email(), row.total(), row.rank());
    }
}
