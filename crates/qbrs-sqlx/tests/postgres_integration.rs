//! End-to-end test against a *real* Postgres. See the design plan's DB-setup
//! decision: PGlite turned out to require a Node.js sidecar to speak the
//! Postgres wire protocol from Rust (its only wire-protocol mode is
//! `@electric-sql/pglite-socket`, a Node package), so it isn't realistically
//! drivable from a Rust test harness. Two supported paths instead:
//!
//! - **No Docker**: `postgresql_embedded` downloads and caches a real,
//!   native Postgres binary on first run (used when `DATABASE_URL` is unset).
//! - **Docker**: `cd examples && docker compose up -d`, then set
//!   `DATABASE_URL=postgres://postgres:postgres@localhost:55432/qbrs_test`.
//!   This is what CI and sandboxed dev environments without open internet
//!   access to the embedded-binary download host should use — that's
//!   exactly the situation this crate's own development environment hit
//!   (DNS for the binary CDN was blocked, while the Docker registry
//!   wasn't), which is precisely the tradeoff the design plan flagged when
//!   picking `postgresql_embedded` over PGlite: no single no-Docker option
//!   is guaranteed to work in every network environment, so both paths are
//!   kept and neither is assumed to always be available.

mod common;

use qbrs::Table;
use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::select::{OrderExt, select};
use qbrs_sqlx::{ExecuteExt, LoadExt, LoadReturningExt};

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
    #[column(default)]
    active: bool,
}

#[derive(Table)]
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

#[tokio::test]
async fn full_crud_roundtrip_against_real_postgres() {
    let (pool, guard) = common::test_pool("qbrs_test").await;

    // Idempotent so this test can run repeatedly against a persistent
    // docker-compose Postgres, not just a fresh throwaway embedded one.
    sqlx::query("DROP TABLE IF EXISTS orders")
        .execute(&pool)
        .await
        .expect("drop orders table");
    sqlx::query("DROP TABLE IF EXISTS users")
        .execute(&pool)
        .await
        .expect("drop users table");

    sqlx::query(
        "CREATE TABLE users (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            email TEXT NOT NULL,
            display_name TEXT,
            active BOOLEAN NOT NULL DEFAULT true
        )",
    )
    .execute(&pool)
    .await
    .expect("create users table");

    sqlx::query(
        "CREATE TABLE orders (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            user_id BIGINT NOT NULL REFERENCES users(id),
            total BIGINT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("create orders table");

    // INSERT .. RETURNING
    let inserted_ids: Vec<i64> = qbrs::insert::insert::<Postgres, _>(users::Table)
        .values(UsersInsert::new("ada@example.com").display_name("Ada"))
        .values(UsersInsert::new("dan@example.com"))
        .returning(users::id)
        .load(&pool)
        .await
        .expect("insert users");
    assert_eq!(inserted_ids.len(), 2);
    let ada_id = inserted_ids[0];
    let dan_id = inserted_ids[1];

    qbrs::insert::insert::<Postgres, _>(orders::Table)
        .values(OrdersInsert::new(ada_id, 1000))
        .values(OrdersInsert::new(ada_id, 2500))
        .execute(&pool)
        .await
        .expect("insert orders");

    // Plain SELECT with WHERE + ORDER BY + LIMIT
    let names: Vec<(String,)> = select((users::email,))
        .from::<Postgres, _>(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::id.asc())
        .limit(10)
        .load(&pool)
        .await
        .expect("select users");
    assert_eq!(
        names,
        vec![
            ("ada@example.com".to_string(),),
            ("dan@example.com".to_string(),)
        ]
    );

    // LEFT JOIN — dan has no orders, so his row's total must come back NULL,
    // proving the join actually reaches Postgres and NULL round-trips.
    let mut rows: Vec<(String, Option<i64>)> = select((users::email, orders::total))
        .from::<Postgres, _>(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("left join select");
    rows.sort();
    assert!(rows.contains(&("dan@example.com".to_string(), None)));
    assert!(rows.contains(&("ada@example.com".to_string(), Some(1000))));
    assert!(rows.contains(&("ada@example.com".to_string(), Some(2500))));

    // UPDATE
    let affected = qbrs::update::update::<Postgres, _>(users::Table)
        .set(UsersUpdate {
            display_name: Some(Some("Ada Lovelace".into())),
            ..Default::default()
        })
        .filter(users::id.eq(ada_id))
        .execute(&pool)
        .await
        .expect("update user");
    assert_eq!(affected, 1);

    let updated_name: Option<(Option<String>,)> = select((users::display_name,))
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(ada_id))
        .load_one(&pool)
        .await
        .expect("select updated user");
    assert_eq!(updated_name, Some((Some("Ada Lovelace".to_string()),)));

    // DELETE
    let deleted = qbrs::delete::delete::<Postgres, _>(users::Table)
        .filter(users::id.eq(dan_id))
        .execute(&pool)
        .await
        .expect("delete user");
    assert_eq!(deleted, 1);

    let remaining: Vec<(i64,)> = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .load(&pool)
        .await
        .expect("select remaining users");
    assert_eq!(remaining, vec![(ada_id,)]);

    common::shutdown(pool, guard).await;
}
