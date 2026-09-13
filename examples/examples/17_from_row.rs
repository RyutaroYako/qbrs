//! `#[derive(FromRow)]`: filling a plain struct from a query result by
//! matching field names. The struct names no column, no table, and no join:
//! nothing ties it to the query that fills it, so it can live in a
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
/// call site listing them: the derive already knows what the table has.
#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct WholeUser {
    id: i64,
    email: String,
    display_name: Option<String>,
    active: bool,
}

/// Both tables have an `id` and both are selected whole, so two fields
/// can't be told apart by name. `from = <column>` fills them by identity
/// instead, using the same key `row.get(users::id)` uses.
#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct UserWithOrder {
    #[from_row(from = users::id)]
    id: i64,
    email: String,
    #[from_row(from = orders::id)]
    order_id: i64,
    total: i64,
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

    // Selected as (id, total, email, display_name), deliberately neither
    // the order nor the exact set `UserSummary` declares. `users::id` is
    // simply not wanted, and the fields are matched by name.
    let summaries: Vec<UserSummary> =
        select((users::id, orders::total, users::email, users::display_name))
            .from(users::Table)
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

    // A different query (no join, different selection order) fills the
    // other struct with nothing said about either of them at the call site.
    let contacts: Vec<Contact> = select((users::email, users::display_name))
        .from(users::Table)
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
    // the schema doesn't leave a query behind. It also counts as one
    // element of the selection tuple however many columns it has.
    let everyone: Vec<WholeUser> = select(users::All)
        .from(users::Table)
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("whole users")
        .into_structs();
    println!("whole rows: {everyone:?}");

    // Two whole tables at once: `email` and `total` are still matched by
    // name, and the two `id`s by the column each one means.
    let joined: Vec<UserWithOrder> = select((users::All, orders::All))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(orders::id.asc())
        .load(&pool)
        .await
        .expect("joined")
        .into_structs();
    println!("user with order: {joined:?}");
    assert!(joined.iter().all(|j| j.order_id > 0 && j.id > 0));

    // When a name doesn't line up, `take` writes the mapping by hand, one
    // field at a time, still moving rather than copying.
    let renamed: Vec<(String, i64)> = select((users::email, orders::total))
        .from(users::Table)
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
