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
///
/// One index parameter per column, always: the index records *where* a
/// column sits in the row, so two columns need two of them. Reusing one
/// `Idx` for both would compile here and then reject every row at the call
/// site, since no row can hold two different columns at the same position.
fn masked_email<Idx, R: users::HasEmail<Idx, Value = String>>(row: &R) -> String {
    let email = row.email();
    let at = email.find('@').unwrap_or(email.len());
    format!("{}***{}", &email[..1], &email[at..])
}

/// Two columns, two indices — and generic over whether the join made
/// `total` nullable, so the same helper serves an INNER and a LEFT join.
fn line<I1, I2, R>(row: &R) -> String
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
        println!("  {}", line(row));
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
