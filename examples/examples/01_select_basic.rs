//! Plain SELECT: filter, order by, limit.
//! Run: `cargo run -p qbrs-examples --example 01_select_basic`

use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::select::{OrderExt, select};
use qbrs_examples::users;
use qbrs_examples::{seed, setup_db};
use qbrs_sqlx::LoadExt;

#[tokio::main]
async fn main() {
    let pool = setup_db().await;
    seed(&pool).await;

    let active_users: Vec<(i64, String, Option<String>)> =
        select((users::id, users::email, users::display_name))
            .from::<Postgres, _>(users::Table)
            .filter(users::active.eq(true))
            .order_by(users::id.asc())
            .limit(10)
            .load(&pool)
            .await
            .expect("select active users");

    println!("Active users (id, email, display_name):");
    for row in &active_users {
        println!("  {row:?}");
    }
    assert_eq!(
        active_users.len(),
        2,
        "seed() creates 3 users, 1 deactivated"
    );
}
