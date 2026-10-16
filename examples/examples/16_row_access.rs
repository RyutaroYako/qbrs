//! What keying a row by column buys over keying it by position: adding a
//! column doesn't move anything, two same-typed columns can't be
//! transposed, and a helper can require one column without knowing what
//! else the query selected.
//!
//! `masked_email` takes any row carrying `users::email`, whatever else it
//! holds: the static equivalent of width subtyping, which a tuple cannot
//! express. `email_and_total` is also generic over whether the join made
//! `total` nullable, so one helper serves an INNER and a LEFT join.
//! `orders::user_id` and `orders::total` are both `i64`, so a positional tuple
//! would let them be swapped with nothing to catch it. Read by column, the
//! order they were selected in never reaches the call site.
//! Run: `cargo run -p qbrs-examples --example 16_row_access`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

fn masked_email<Idx, R: users::HasEmail<Idx, Value = String>>(row: &R) -> String {
    let email = row.email();
    let at = email.find('@').unwrap_or(email.len());
    format!("{}***{}", &email[..1], &email[at..])
}

// Not one shared index for both bounds: an index records where a column sits,
// and no row holds two columns at one position, so every call site would fail.
fn email_and_total<I1, I2, R>(row: &R) -> String
where
    R: users::HasEmail<I1, Value = String> + orders::HasTotal<I2>,
    <R as orders::HasTotal<I2>>::Value: std::fmt::Debug,
{
    format!("{} {:?}", row.email(), row.total())
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let two_columns = select((users::email, users::active))
        .from(users::Table)
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("two-column select");

    let four_columns = select((users::id, users::email, users::display_name, orders::total))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("four-column select");

    for row in &two_columns {
        println!("{} active={}", masked_email(row), row.active());
    }
    for row in &four_columns {
        println!("{} total={:?}", masked_email(row), row.total());
        println!("  {}", email_and_total(row));
    }

    let orders_rows = select((orders::total, orders::user_id))
        .from(orders::Table)
        .order_by(orders::id.asc())
        .load(&pool)
        .await
        .expect("orders select");
    for row in &orders_rows {
        println!(
            "user_id={} total={}",
            row.get(orders::user_id),
            row.get(orders::total),
        );
    }
    assert!(
        orders_rows
            .iter()
            .all(|row| row.get(orders::total) > row.get(orders::user_id)),
        "selected as (total, user_id), each still reads back as its own column"
    );
}
