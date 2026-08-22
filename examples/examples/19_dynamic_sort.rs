//! A `?sort=`/`?group=` parameter: sort and grouping keys that aren't known
//! until runtime. `sort_key(..)`/`grouping(..)` discharge a key's scope
//! requirement the way `predicate(..)` does for a condition, so keys naming
//! different tables can share one collection.
//! Run: `cargo run -p qbrs-examples --example 19_dynamic_sort`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

/// What a request would have parsed into.
struct ListParams {
    sort: Vec<(&'static str, SortDir)>,
    group_by_user: bool,
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let params = ListParams {
        sort: vec![("total", SortDir::Desc), ("email", SortDir::Asc)],
        group_by_user: false,
    };

    // Each key is checked against the query's scope as it is built, then
    // carries that proof instead of its own table list — which is what lets
    // `users` and `orders` keys sit in one `Vec`.
    let keys: Vec<_> = params
        .sort
        .iter()
        .map(|(column, dir)| match *column {
            "email" => sort_key(users::email.sort(*dir)),
            _ => sort_key(orders::total.sort(*dir)),
        })
        .collect();

    let rows = select((users::email, orders::total))
        .from::<Postgres, _>(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .order_by_all(keys)
        .load(&pool)
        .await
        .expect("sorted orders");

    println!("orders, sorted as the request asked:");
    for row in &rows {
        println!("  {} {}", row.email(), row.total());
    }

    // The same for `GROUP BY`, and a single key goes into the singular
    // method unchanged — `.order_by`/`.group_by` take either form.
    let mut dimensions = vec![grouping(users::email)];
    if params.group_by_user {
        dimensions.push(grouping(users::id));
    }

    let totals = select((users::email, sum(orders::total)))
        .from::<Postgres, _>(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .group_by_all(dimensions)
        .order_by(sort_key(users::email.asc()))
        .load(&pool)
        .await
        .expect("totals per user");

    println!("totals per user:");
    for row in &totals {
        println!("  {} {:?}", row.email(), row.get(sum(orders::total)));
    }
}
