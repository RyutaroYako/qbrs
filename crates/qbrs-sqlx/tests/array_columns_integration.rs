mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;
use sqlx::PgPool;

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

#[derive(qbrs::FromRow, Debug, PartialEq)]
struct Methods {
    login_methods: Vec<String>,
    notify: Option<Vec<String>>,
}

type DelaysVersionsAndNotify = (Vec<i32>, Vec<i64>, Option<Vec<String>>);

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

    let filled_row = {
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
        .values(filled_row)
        .returning(accounts::id)
        .load_one(&pool)
        .await
        .expect("insert a row of arrays")
        .expect("one row");

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
    let empty_id: i64 = qbrs::insert::insert(accounts::Table)
        .values(empty_row)
        .returning(accounts::id)
        .load_one(&pool)
        .await
        .expect("insert empty arrays and a NULL one")
        .expect("one row");

    array_columns_fill_a_from_row_struct_by_name(&pool, id).await;
    an_empty_array_reads_back_as_an_empty_vec_apart_from_a_null_array(&pool).await;
    an_array_column_equals_a_bound_array_compared_as_a_whole(&pool).await;
    an_update_replaces_an_array_column_whole(&pool, id).await;
    #[cfg(feature = "uuid")]
    a_uuid_array_column_decodes_as_a_vec_of_uuid(&pool).await;
    eq_any_finds_the_rows_whose_array_column_holds_a_value(&pool, id).await;
    negated_eq_any_finds_the_rows_whose_array_column_holds_none_of_it(&pool, empty_id).await;
    eq_any_over_a_null_array_is_null_so_its_row_is_in_neither_count(&pool).await;
    eq_any_against_a_bound_array_matches_a_column_in_one_parameter(&pool, id).await;

    common::shutdown(pool, guard).await;
}

type AccountsScope = Cons<TableSlot<accounts::Table, NotNull>, Nil>;

async fn count_accounts_where(pool: &PgPool, condition: Predicate<Postgres, AccountsScope>) -> i64 {
    select(qbrs::expr::count())
        .from(accounts::Table)
        .filter(condition)
        .load_one(pool)
        .await
        .expect("count the matching accounts")
        .expect("one row")
}

async fn ids_of_accounts_where(
    pool: &PgPool,
    condition: Predicate<Postgres, AccountsScope>,
) -> Vec<i64> {
    select(accounts::id)
        .from(accounts::Table)
        .filter(condition)
        .order_by(accounts::id.asc())
        .load(pool)
        .await
        .expect("select the matching accounts")
}

async fn array_columns_fill_a_from_row_struct_by_name(pool: &PgPool, id: i64) {
    let filled: Methods = select(accounts::All)
        .from(accounts::Table)
        .filter(accounts::id.eq(id))
        .load_one(pool)
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
}

async fn an_empty_array_reads_back_as_an_empty_vec_apart_from_a_null_array(pool: &PgPool) {
    let rows: Vec<DelaysVersionsAndNotify> = select((
        accounts::retry_delays,
        accounts::seen_versions,
        accounts::notify,
    ))
    .from(accounts::Table)
    .order_by(accounts::id.asc())
    .load(pool)
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
}

async fn an_array_column_equals_a_bound_array_compared_as_a_whole(pool: &PgPool) {
    let matched =
        count_accounts_where(pool, predicate(accounts::retry_delays.eq(vec![1i32, 2, 5]))).await;
    assert_eq!(matched, 1);
}

async fn an_update_replaces_an_array_column_whole(pool: &PgPool, id: i64) {
    let changed = qbrs::update::update(accounts::Table)
        .set(
            qbrs::update::Assignments::from_row(AccountsUpdate {
                login_methods: Some(vec!["sso".to_string()]),
                ..Default::default()
            })
            .expect("login_methods is set"),
        )
        .filter(accounts::id.eq(id))
        .execute(pool)
        .await
        .expect("update an array column");
    assert_eq!(changed, 1);
    let replaced: Vec<String> = select(accounts::login_methods)
        .from(accounts::Table)
        .filter(accounts::id.eq(id))
        .load_one(pool)
        .await
        .expect("read the updated array back")
        .expect("one row");
    assert_eq!(replaced, vec!["sso".to_string()]);
}

#[cfg(feature = "uuid")]
async fn a_uuid_array_column_decodes_as_a_vec_of_uuid(pool: &PgPool) {
    let denied: Vec<Vec<uuid::Uuid>> = select(accounts::denied)
        .from(accounts::Table)
        .order_by(accounts::id.asc())
        .load(pool)
        .await
        .expect("read the uuid array");
    assert_eq!(denied, vec![vec![uuid::Uuid::nil()], vec![]]);
}

async fn eq_any_finds_the_rows_whose_array_column_holds_a_value(pool: &PgPool, sso_id: i64) {
    let referencing = ids_of_accounts_where(
        pool,
        predicate("sso".to_string().eq_any(accounts::login_methods)),
    )
    .await;
    assert_eq!(referencing, vec![sso_id]);
}

async fn negated_eq_any_finds_the_rows_whose_array_column_holds_none_of_it(
    pool: &PgPool,
    empty_id: i64,
) {
    let others = ids_of_accounts_where(
        pool,
        predicate(!"sso".to_string().eq_any(accounts::login_methods)),
    )
    .await;
    assert_eq!(others, vec![empty_id]);
}

async fn eq_any_over_a_null_array_is_null_so_its_row_is_in_neither_count(pool: &PgPool) {
    let notified = count_accounts_where(
        pool,
        predicate("ops@example.com".to_string().eq_any(accounts::notify)),
    )
    .await;
    let not_notified = count_accounts_where(
        pool,
        predicate(!"ops@example.com".to_string().eq_any(accounts::notify)),
    )
    .await;
    assert_eq!((notified, not_notified), (1, 0));
}

async fn eq_any_against_a_bound_array_matches_a_column_in_one_parameter(pool: &PgPool, id: i64) {
    let by_id: Vec<i64> = select(accounts::id)
        .from(accounts::Table)
        .filter(accounts::id.eq_any(vec![id, id + 1000]))
        .load(pool)
        .await
        .expect("membership in a bound array");
    assert_eq!(by_id, vec![id]);
}
