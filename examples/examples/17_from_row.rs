//! `#[derive(FromRow)]`: filling a plain struct from a query result by
//! matching field names. The struct names no column, no table, and no join —
//! nothing that ties it to the query that fills it, so it can live in a
//! domain module and carry `#[derive(Serialize)]` for an API response.
//! Run: `cargo run -p qbrs-examples --example 17_from_row`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct UserSummary {
    email: String,
    display_name: Option<String>,
    total: Option<i64>,
}

/// Every column of `users`, which `select(users::All)` fills without the
/// call site listing them — the derive already knows what the table has.
#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct WholeUser {
    id: i64,
    email: String,
    display_name: Option<String>,
    active: bool,
}

/// A different view, declared independently: fewer fields, opposite order.
#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct Contact {
    display_name: Option<String>,
    email: String,
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // Selected as (id, total, email, display_name) — deliberately neither
    // the order nor the exact set `UserSummary` declares. `users::id` is
    // simply not wanted, and the fields are matched by name.
    let summaries: Vec<UserSummary> =
        select((users::id, orders::total, users::email, users::display_name))
            .from::<Postgres, _>(users::Table)
            .left_join(orders::Table, orders::user_id.eq(users::id))
            .order_by(users::id.asc())
            .load(&pool)
            .await
            .expect("summaries")
            .into_structs();

    println!("summaries:");
    for s in &summaries {
        println!("  {s:?}");
    }
    // `total` is `Option<i64>` because of the LEFT JOIN, and the struct has
    // to agree: declaring it `i64` would be a compile error, not a surprise.
    assert!(
        summaries
            .iter()
            .any(|s| s.email == "dan@example.com" && s.total.is_none())
    );

    // A different query — no join, different selection order — fills the
    // other struct with nothing said about either of them at the call site.
    let contacts: Vec<Contact> = select((users::email, users::display_name))
        .from::<Postgres, _>(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("contacts")
        .into_structs();

    println!("contacts:");
    for c in &contacts {
        println!("  {c:?}");
    }

    // `users::All` is the table's own column list, so adding a column to
    // the schema doesn't leave a query behind — and it counts as one
    // element of the selection tuple however many columns it has.
    let everyone: Vec<WholeUser> = select(users::All)
        .from::<Postgres, _>(users::Table)
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("whole users")
        .into_structs();
    println!("whole rows: {everyone:?}");

    // When a name doesn't line up, `take` writes the mapping by hand — one
    // field at a time, still moving rather than copying.
    let renamed: Vec<(String, i64)> = select((users::email, orders::total))
        .from::<Postgres, _>(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .load(&pool)
        .await
        .expect("renamed")
        .into_iter()
        .map(|row| {
            let (who, row) = row.take(users::email);
            let (amount, _) = row.take(orders::total);
            (who, amount)
        })
        .collect();
    println!("hand-mapped: {renamed:?}");
}
