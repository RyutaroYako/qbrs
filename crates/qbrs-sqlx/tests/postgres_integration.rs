//! End-to-end test against a *real* Postgres.
//!
//! - **Default (no setup)**: with `DATABASE_URL` unset, a throwaway
//!   PostgreSQL 17.5 runs in a temp directory and is torn down after. The
//!   engine is linked in at build time, so nothing is downloaded at test
//!   time.
//! - **External Postgres**: set `DATABASE_URL` to run the same tests against
//!   a real server.

mod common;

use qbrs::Table;
use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::row::{IntoStructs, IntoTuples};
use qbrs::select::{OrderExt, select};
use qbrs::statement::Statement;
use qbrs_sqlx::{ExecuteExt, LoadExt};

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

/// The same shape `#[derive(Table)]` describes, filled by name from a
/// `select(users::All)` row.
#[derive(qbrs::FromRow, Debug)]
#[allow(dead_code)]
struct UserRow {
    id: i64,
    email: String,
    display_name: Option<String>,
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
    // `DATABASE_URL` Postgres, not just a fresh throwaway embedded one.
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
        .values(
            UsersInsert::builder()
                .email("ada@example.com")
                .display_name("Ada")
                .build(),
        )
        .values(UsersInsert::builder().email("dan@example.com").build())
        .returning(users::id)
        .load(&pool)
        .await
        .expect("insert users");
    assert_eq!(inserted_ids.len(), 2);
    let ada_id = inserted_ids[0];
    let dan_id = inserted_ids[1];

    qbrs::insert::insert::<Postgres, _>(orders::Table)
        .values(OrdersInsert::builder().user_id(ada_id).total(1000).build())
        .values(OrdersInsert::builder().user_id(ada_id).total(2500).build())
        .execute(&pool)
        .await
        .expect("insert orders");

    // Plain SELECT with WHERE + ORDER BY + LIMIT
    let names: Vec<String> = select(users::email)
        .from::<Postgres, _>(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::id.asc())
        .limit(10)
        .load(&pool)
        .await
        .expect("select users");
    assert_eq!(
        names,
        vec!["ada@example.com".to_string(), "dan@example.com".to_string()]
    );

    // LEFT JOIN — dan has no orders, so his row's total must come back NULL,
    // proving the join actually reaches Postgres and NULL round-trips.
    let mut rows: Vec<(String, Option<i64>)> = select((users::email, orders::total))
        .from::<Postgres, _>(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(users::id.asc())
        .load(&pool)
        .await
        .expect("left join select")
        .into_tuples();
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
        .expect("display_name is set")
        .filter(users::id.eq(ada_id))
        .execute(&pool)
        .await
        .expect("update user");
    assert_eq!(affected, 1);

    let updated_name: Option<Option<String>> = select(users::display_name)
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(ada_id))
        .load_one(&pool)
        .await
        .expect("select updated user");
    assert_eq!(updated_name, Some(Some("Ada Lovelace".to_string())));

    // Whole-table selection, decoded into the schema struct itself: the
    // column list is the derive's, so it cannot drift from the table.
    let whole: Vec<UserRow> = select(users::All)
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(ada_id))
        .load(&pool)
        .await
        .expect("select every column")
        .into_structs();
    assert_eq!(whole.len(), 1);
    assert_eq!(whole[0].email, "ada@example.com");
    assert_eq!(whole[0].display_name.as_deref(), Some("Ada Lovelace"));

    // The nullable side of a LEFT JOIN reaches the row as `Option`, one
    // whole table at a time.
    let joined = select((users::email, orders::All))
        .from::<Postgres, _>(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::id.eq(dan_id))
        .load_one(&pool)
        .await
        .expect("left join every order column")
        .expect("dan is there");
    assert_eq!(joined.get(users::email), "dan@example.com");
    assert_eq!(joined.get(orders::total), &None);

    // DELETE
    let deleted = qbrs::delete::delete::<Postgres, _>(users::Table)
        .filter(users::id.eq(dan_id))
        .execute(&pool)
        .await
        .expect("delete user");
    assert_eq!(deleted, 1);

    let remaining: Vec<i64> = select(users::id)
        .from::<Postgres, _>(users::Table)
        .load(&pool)
        .await
        .expect("select remaining users");
    assert_eq!(remaining, vec![ada_id]);

    common::shutdown(pool, guard).await;
}
