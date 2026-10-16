//! `#[derive(FromRow)]`: filling a plain struct from a query result by
//! matching field names. The struct names no column, no table, and no join:
//! nothing ties it to the query that fills it, so it can live in a
//! domain module and carry `#[derive(Serialize)]` for an API response.
//!
//! A selection need not match a struct's field order or exact field set, so
//! two independent queries fill `UserSummary` and `Contact` with nothing said
//! at the call site. `select(users::All)` is the table's own column list, so
//! adding a column to the schema leaves no query behind, and it counts as one
//! element of a selection tuple. Where a name does not line up, `take` maps
//! one field at a time by hand, still moving rather than copying.
//! Run: `cargo run -p qbrs-examples --example 17_from_row`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct UserSummary {
    email: String,
    display_name: Option<String>,
    // Not `i64`: the LEFT JOIN that fills it makes `total` nullable, and a
    // non-`Option` field is a compile error.
    total: Option<i64>,
}

#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct WholeUser {
    id: i64,
    email: String,
    display_name: Option<String>,
    active: bool,
}

// Not matched by name: both tables have an `id` and both are selected whole,
// so `from = <column>` fills each by identity, the key `row.get(..)` uses.
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
    assert!(
        summaries
            .iter()
            .any(|s| s.email == "dan@example.com" && s.total.is_none()),
        "Dan has no orders, so his LEFT JOINed total is None"
    );

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

    let everyone: Vec<WholeUser> = select(users::All)
        .from(users::Table)
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("whole users")
        .into_structs();
    println!("whole rows: {everyone:?}");

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
