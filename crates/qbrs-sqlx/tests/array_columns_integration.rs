//! Postgres array columns, end to end: declared in a schema, bound, and
//! decoded back into the `Vec<T>` the schema names. An array is one bind
//! parameter rather than a rendered list, so what a database does with it
//! is the only thing that says the mapping is right.

mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "accounts")]
#[allow(dead_code)]
struct Accounts {
    #[column(primary_key, generated)]
    id: i64,
    login_methods: Vec<String>,
    retry_delays: Vec<i32>,
    seen_versions: Vec<i64>,
    notify: Option<Vec<String>>,
    #[cfg(feature = "uuid")]
    denied: Vec<uuid::Uuid>,
}

/// The DTO shape a handler returns, filled by name.
#[derive(qbrs::FromRow, Debug, PartialEq)]
struct Methods {
    login_methods: Vec<String>,
    notify: Option<Vec<String>>,
}

/// The three arrays a row selects, in selection order.
type Arrays = (Vec<i32>, Vec<i64>, Option<Vec<String>>);

#[tokio::test]
async fn array_columns_bind_and_decode_as_the_vec_the_schema_names() {
    let (pool, guard) = common::test_pool("qbrs_arrays").await;

    sqlx::query("DROP TABLE IF EXISTS accounts")
        .execute(&pool)
        .await
        .expect("drop accounts");
    sqlx::query(
        "CREATE TABLE accounts (
            id BIGSERIAL PRIMARY KEY,
            login_methods TEXT[] NOT NULL,
            retry_delays INTEGER[] NOT NULL,
            seen_versions BIGINT[] NOT NULL,
            notify TEXT[],
            denied UUID[] NOT NULL DEFAULT '{}'
        )",
    )
    .execute(&pool)
    .await
    .expect("create accounts");

    let row = || {
        let builder = AccountsInsert::builder()
            .login_methods(vec!["password".to_string(), "sso".to_string()])
            .retry_delays(vec![1i32, 2, 5])
            .seen_versions(vec![10i64, 11])
            .notify(Some(vec!["ops@example.com".to_string()]));
        #[cfg(feature = "uuid")]
        let builder = builder.denied(vec![uuid::Uuid::nil()]);
        builder.build()
    };

    let id: i64 = qbrs::insert::insert(accounts::Table)
        .values(row())
        .returning(accounts::id)
        .load_one(&pool)
        .await
        .expect("insert a row of arrays")
        .expect("one row");

    // An empty array is a value, not a NULL, and the two stay apart.
    let empty_row = {
        let builder = AccountsInsert::builder()
            .login_methods(Vec::<String>::new())
            .retry_delays(Vec::<i32>::new())
            .seen_versions(Vec::<i64>::new())
            .notify(None::<Vec<String>>);
        #[cfg(feature = "uuid")]
        let builder = builder.denied(Vec::<uuid::Uuid>::new());
        builder.build()
    };
    qbrs::insert::insert(accounts::Table)
        .values(empty_row)
        .execute(&pool)
        .await
        .expect("insert empty arrays and a NULL one");

    let filled: Methods = select(accounts::All)
        .from(accounts::Table)
        .filter(accounts::id.eq(id))
        .load_one(&pool)
        .await
        .expect("read the arrays back")
        .expect("one row")
        .into_struct();
    assert_eq!(
        filled,
        Methods {
            login_methods: vec!["password".to_string(), "sso".to_string()],
            notify: Some(vec!["ops@example.com".to_string()]),
        }
    );

    let rows: Vec<Arrays> = select((
        accounts::retry_delays,
        accounts::seen_versions,
        accounts::notify,
    ))
    .from(accounts::Table)
    .order_by(accounts::id.asc())
    .load(&pool)
    .await
    .expect("read every array column")
    .into_tuples();
    assert_eq!(
        rows,
        vec![
            (
                vec![1, 2, 5],
                vec![10, 11],
                Some(vec!["ops@example.com".to_string()])
            ),
            (vec![], vec![], None),
        ]
    );

    // An array compares as a whole, which is what `Comparable<T> for T`
    // gives every leaf type — and Postgres agrees for arrays.
    let matched: i64 = select(qbrs::expr::count())
        .from(accounts::Table)
        .filter(accounts::retry_delays.eq(vec![1i32, 2, 5]))
        .load_one(&pool)
        .await
        .expect("compare an array against a bound one")
        .expect("one row");
    assert_eq!(matched, 1);

    // `UPDATE .. SET <array column> = <array>` replaces the whole value.
    let changed = qbrs::update::update(accounts::Table)
        .set(
            qbrs::update::Assignments::from_row(AccountsUpdate {
                login_methods: Some(vec!["sso".to_string()]),
                ..Default::default()
            })
            .expect("login_methods is set"),
        )
        .filter(accounts::id.eq(id))
        .execute(&pool)
        .await
        .expect("update an array column");
    assert_eq!(changed, 1);
    let replaced: Vec<String> = select(accounts::login_methods)
        .from(accounts::Table)
        .filter(accounts::id.eq(id))
        .load_one(&pool)
        .await
        .expect("read the updated array back")
        .expect("one row");
    assert_eq!(replaced, vec!["sso".to_string()]);

    #[cfg(feature = "uuid")]
    {
        let denied: Vec<Vec<uuid::Uuid>> = select(accounts::denied)
            .from(accounts::Table)
            .order_by(accounts::id.asc())
            .load(&pool)
            .await
            .expect("read the uuid array");
        assert_eq!(denied, vec![vec![uuid::Uuid::nil()], vec![]]);
    }

    common::shutdown(pool, guard).await;
}
