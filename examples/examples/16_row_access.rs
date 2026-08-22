//! What keying a row by column buys over keying it by position: adding a
//! column doesn't move anything, two same-typed columns can't be
//! transposed, and a helper can require one column without knowing what
//! else the query selected.
//! Run: `cargo run -p qbrs-examples --example 16_row_access`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::LoadExt;

/// Takes any row carrying `users::email`, whatever else it holds — the
/// static equivalent of width subtyping, which a tuple can't express.
/// `Idx` is the inferred lookup index; callers never write it.
fn masked_email<Idx, R: users::HasEmail<Idx, Value = String>>(row: &R) -> String {
    let email = row.email();
    let at = email.find('@').unwrap_or(email.len());
    format!("{}***{}", &email[..1], &email[at..])
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let two_columns = select((users::email, users::active))
        .from::<Postgres, _>(users::Table)
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("two-column select");

    // The same helper accepts a wider row from an entirely different query.
    let four_columns = select((users::id, users::email, users::display_name, orders::total))
        .from::<Postgres, _>(users::Table)
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
    }

    // `orders::user_id` and `orders::total` are both `i64`, so a positional
    // tuple would let them be swapped with nothing to catch it. Read by
    // column, the order they were selected in doesn't reach the call site.
    let orders_rows = select((orders::total, orders::user_id))
        .from::<Postgres, _>(orders::Table)
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
    // If the two `i64` columns had been read positionally and transposed,
    // this would hold the other way round.
    assert!(
        orders_rows
            .iter()
            .all(|row| row.get(orders::total) > row.get(orders::user_id))
    );
}
