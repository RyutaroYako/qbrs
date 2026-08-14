//! Dynamic query composition *without* a Drizzle-style `.$dynamic()`
//! escape hatch: `.filter()` doesn't change the builder's type, so it can
//! be called conditionally, in a loop, or from a shared helper function.
//! Run: `cargo run -p qbrs-examples --example 07_dynamic_filters`

use qbrs::dialect::Postgres;
use qbrs::expr::{ExprMethods, TextExprMethods};
use qbrs::scope::Find;
use qbrs::select::{Select, select};
use qbrs_examples::{seed, setup_db, users};
use qbrs_sqlx::LoadExt;

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
            select((users::email,)).from::<Postgres, _>(users::Table),
            &search,
        );
        let rows: Vec<(String,)> = query.load(&pool).await.expect("search users");
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
    let mut query = select((users::email,)).from::<Postgres, _>(users::Table);
    for cond in candidate_filters.into_iter().flatten() {
        query = query.filter(cond);
    }
    let rows: Vec<(String,)> = query.load(&pool).await.expect("looped filters");
    println!("looped-filter result: {rows:?}");
}
