//! Dynamic query composition *without* a Drizzle-style `.$dynamic()`
//! escape hatch: `.filter()` doesn't change the builder's type, so it can
//! be called conditionally, in a loop, or from a shared helper function.
//!
//! `apply_search` is generic over any scope that contains `users::Table`, so
//! it never needs to know what else is joined into the query it is given. A
//! WHERE clause built from a runtime-length list is a plain loop over
//! `.filter()`, which may run any number of times, zero included. A loop ANDs.
//! `any_of` folds a runtime-length OR instead, and `Predicate::any_of` does
//! the same across tables. `.count()` reruns the same FROM/JOIN/WHERE without
//! ORDER BY/LIMIT/OFFSET, so a paginated endpoint reports its total without
//! cloning the query.
//! Run: `cargo run -p qbrs-examples --example 07_dynamic_filters`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

struct UserSearch {
    email_contains: Option<String>,
    active_only: bool,
}

fn apply_search<D, Scope, Sel, Idx>(
    mut query: Select<D, Scope, Sel>,
    search: &UserSearch,
) -> Select<D, Scope, Sel>
where
    Scope: Find<users::Table, Idx>,
{
    if let Some(needle) = &search.email_contains {
        query = query.filter(users::email.like(format!("%{needle}%")));
    }
    if search.active_only {
        query = query.filter(users::active.eq(true));
    }
    query
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    for search in [
        UserSearch {
            email_contains: None,
            active_only: false,
        },
        UserSearch {
            email_contains: Some("ada".into()),
            active_only: false,
        },
        UserSearch {
            email_contains: None,
            active_only: true,
        },
    ] {
        let query = apply_search(select(users::email).from(users::Table), &search);
        let rows: Vec<String> = query.load(&pool).await.expect("search users");
        println!(
            "email_contains={:?} active_only={} -> {rows:?}",
            search.email_contains, search.active_only
        );
    }

    let candidate_filters = vec![Some(users::active.eq(true)), None];
    let mut query = select(users::email).from(users::Table);
    for cond in candidate_filters.into_iter().flatten() {
        query = query.filter(cond);
    }
    let rows: Vec<String> = query.load(&pool).await.expect("looped filters");
    println!("looped-filter result: {rows:?}");

    // Not a `Vec` of bare `Expr`s: the tables an expression references are
    // part of its type, so conditions on two tables cannot share one.
    let mut conds = Vec::new();
    conds.push(predicate(users::active.eq(true)));
    if true {
        conds.push(predicate(orders::total.gt(1000i64)));
    }
    let big: Vec<String> = select(users::email)
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .filter_all(conds)
        .load(&pool)
        .await
        .expect("collected predicates");
    println!("active users with a big order: {big:?}");

    // Not a hand-folded `.or()` chain: each pair widens the tables the
    // expression claims, so the fold has no single type to accumulate in.
    let terms = ["ada", "grace"];
    let searched: Vec<String> = select(users::email)
        .from(users::Table)
        .filter(any_of(
            terms
                .iter()
                .map(|t| sql!(Bool, "? LIKE ?", users::email, format!("{t}%"))),
        ))
        .load(&pool)
        .await
        .expect("searched");
    println!("search hits: {searched:?}");

    let any_signal: Vec<String> = select(users::email)
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .filter(Predicate::any_of([
            predicate(users::active.eq(false)),
            predicate(orders::total.gt(2000i64)),
        ]))
        .load(&pool)
        .await
        .expect("either signal");
    println!("inactive or big-spending: {any_signal:?}");

    let page = select(users::email)
        .from(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::id.asc())
        .limit(1u32);
    println!(
        "page of {} shows {:?}",
        page.count(&pool).await.expect("total"),
        page.load(&pool).await.expect("page")
    );
}
