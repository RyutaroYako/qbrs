//! Dynamic query composition *without* a Drizzle-style `.$dynamic()`
//! escape hatch: `.filter()` doesn't change the builder's type, so it can
//! be called conditionally, in a loop, or from a shared helper function.
//! Run: `cargo run -p qbrs-examples --example 07_dynamic_filters`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

/// A search form's optional fields — in a real app these would come from
/// query-string params, most of them usually absent.
struct UserSearch {
    email_contains: Option<String>,
    active_only: bool,
}

/// Generic over *any* scope that contains `users::Table` — this helper
/// doesn't need to know what else might be joined into the query it's
/// given, unlike a hand-rolled equivalent tied to one concrete join shape.
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
        let query = apply_search(
            select(users::email).from::<Postgres, _>(users::Table),
            &search,
        );
        let rows: Vec<String> = query.load(&pool).await.expect("search users");
        println!(
            "email_contains={:?} active_only={} -> {rows:?}",
            search.email_contains, search.active_only
        );
    }

    // Building a WHERE clause from a runtime-length list of conditions:
    // `.filter()` is called once per present condition in a plain loop —
    // the builder's type never changes, so an arbitrary (including zero)
    // number of iterations is fine.
    let candidate_filters = vec![Some(users::active.eq(true)), None];
    let mut query = select(users::email).from::<Postgres, _>(users::Table);
    for cond in candidate_filters.into_iter().flatten() {
        query = query.filter(cond);
    }
    let rows: Vec<String> = query.load(&pool).await.expect("looped filters");
    println!("looped-filter result: {rows:?}");

    // Conditions from *different* tables can't share one `Expr` type — the
    // tables an expression references are part of it. `predicate(..)`
    // discharges that requirement against the query's scope, so a collection
    // of them is buildable and passable.
    let mut conds = Vec::new();
    conds.push(predicate(users::active.eq(true)));
    if true {
        conds.push(predicate(orders::total.gt(1000i64)));
    }
    let big: Vec<String> = select(users::email)
        .from::<Postgres, _>(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .filter_all(conds)
        .load(&pool)
        .await
        .expect("collected predicates");
    println!("active users with a big order: {big:?}");

    // A loop ANDs. For a runtime-length OR — a search box with N terms —
    // `any_of` folds the collection instead; folding `.or()` by hand can't
    // type-check, since each pair widens the tables the expression claims.
    let terms = ["ada", "grace"];
    let searched: Vec<String> = select(users::email)
        .from::<Postgres, _>(users::Table)
        .filter(any_of(
            terms
                .iter()
                .map(|t| sql!(Bool, "? LIKE ?", users::email, format!("{t}%"))),
        ))
        .load(&pool)
        .await
        .expect("searched");
    println!("search hits: {searched:?}");

    // Across tables, the same shape goes through `Predicate`, which `.filter`
    // takes as readily as an `Expr`.
    let any_signal: Vec<String> = select(users::email)
        .from::<Postgres, _>(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .filter(Predicate::any([
            predicate(users::active.eq(false)),
            predicate(orders::total.gt(2000i64)),
        ]))
        .load(&pool)
        .await
        .expect("either signal");
    println!("inactive or big-spending: {any_signal:?}");

    // `.count()` answers "how many rows would this return" — the same
    // FROM/JOIN/WHERE, with any ORDER BY/LIMIT/OFFSET ignored, so a
    // paginated endpoint can report a total without cloning the query.
    let page = select(users::email)
        .from::<Postgres, _>(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::id.asc())
        .limit(1u32);
    println!(
        "page of {} shows {:?}",
        page.count(&pool).await.expect("total"),
        page.load(&pool).await.expect("page")
    );
}
