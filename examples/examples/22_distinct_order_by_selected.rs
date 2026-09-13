//! `.order_by_selected(..)`/`.order_by_selection(..)`: `SELECT DISTINCT`
//! requires its sort key to be in the selection (Postgres rejects one that
//! isn't), so these check that at compile time instead of at the database.
//! Plain `.order_by(..)` still works for a non-`DISTINCT` query, since it
//! may sort by any column in scope, not only a selected one.
//! Run: `cargo run -p qbrs-examples --example 22_distinct_order_by_selected`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // Every user who's placed an order, deduplicated by the join.
    // `.order_by_selected` only accepts `users::email` because it's one of
    // the two selected columns. `users::id`, in scope but not selected,
    // would be a compile error here.
    let buyers: Vec<(String, bool)> = select((users::email, users::active))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .distinct()
        .order_by_selected(users::email, SortDir::Asc)
        .load(&pool)
        .await
        .expect("distinct buyers")
        .into_tuples();
    println!("buyers, deduplicated and sorted: {buyers:?}");

    // A single un-tupled selection has nothing to name, so
    // `.order_by_selection` sorts by "the one selected column" with no key to
    // pass.
    let emails: Vec<String> = select(users::email)
        .from(users::Table)
        .distinct()
        .order_by_selection(SortDir::Desc)
        .load(&pool)
        .await
        .expect("distinct emails");
    println!("distinct emails, descending: {emails:?}");

    // The argument names a *field*, not an expression, so a computed column
    // is sorted by its label rather than by respelling it, and the clause
    // renders the selected expression the label found.
    label!(spend);

    let by_spend: Vec<(String, i64)> = select((users::email, orders::total.label(label::spend)))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .distinct()
        .order_by_selected(label::spend, SortDir::Desc)
        .load(&pool)
        .await
        .expect("distinct spend")
        .into_tuples();
    println!("distinct (email, spend), biggest first: {by_spend:?}");
}
