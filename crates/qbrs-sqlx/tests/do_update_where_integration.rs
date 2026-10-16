mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;
use sqlx::PgPool;

#[derive(Table)]
#[table(name = "grants")]
#[allow(dead_code)]
struct Grants {
    #[column(primary_key, generated)]
    id: i64,
    account: String,
    authorization_url: Option<String>,
}

fn take_the_url_unless_one_is_held() -> ConflictUpdate<Postgres, grants::Table> {
    ConflictUpdate::set_to(
        grants::authorization_url,
        excluded(grants::authorization_url),
    )
    .filter(grants::authorization_url.is_null())
}

fn grant(account: &'static str, url: &'static str) -> GrantsInsert {
    GrantsInsert::builder()
        .account(account)
        .authorization_url(url)
        .build()
}

fn grant_once(url: &'static str) -> Insert<Postgres, GrantsInsert> {
    qbrs::insert::insert(grants::Table)
        .values(grant("ada", url))
        .on_conflict_do_update(grants::account, take_the_url_unless_one_is_held())
}

#[tokio::test]
async fn a_conditional_do_update_leaves_the_rows_it_rejects_unwritten_and_uncounted() {
    let (pool, guard) = common::test_pool("qbrs_do_update_where").await;

    sqlx::query("DROP TABLE IF EXISTS grants")
        .execute(&pool)
        .await
        .expect("drop grants");
    sqlx::query(
        "CREATE TABLE grants (
            id BIGSERIAL PRIMARY KEY,
            account TEXT NOT NULL UNIQUE,
            authorization_url TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("create grants");

    assert_eq!(
        grant_once("https://first")
            .execute(&pool)
            .await
            .expect("the first grant"),
        1
    );

    a_conflicting_row_the_condition_rejects_is_neither_written_nor_counted(&pool).await;
    the_condition_is_read_per_statement_so_clearing_the_url_reopens_the_grant(&pool).await;
    a_partial_index_target_and_a_conditional_update_execute_in_one_statement(&pool).await;
    a_multi_row_upsert_counts_rows_written_rather_than_the_branch_each_took(&pool).await;

    common::shutdown(pool, guard).await;
}

async fn a_conflicting_row_the_condition_rejects_is_neither_written_nor_counted(pool: &PgPool) {
    assert_eq!(
        grant_once("https://second")
            .execute(pool)
            .await
            .expect("the second grant"),
        0
    );

    let held: Option<String> = select(grants::authorization_url)
        .from(grants::Table)
        .load_one(pool)
        .await
        .expect("read the row back")
        .expect("one row");
    assert_eq!(held, Some("https://first".to_string()));
}

async fn the_condition_is_read_per_statement_so_clearing_the_url_reopens_the_grant(pool: &PgPool) {
    qbrs::update::update(grants::Table)
        .set_to(grants::authorization_url, qbrs::expr::null())
        .execute(pool)
        .await
        .expect("clear the url");
    assert_eq!(
        grant_once("https://third")
            .execute(pool)
            .await
            .expect("the third grant"),
        1
    );
}

async fn a_partial_index_target_and_a_conditional_update_execute_in_one_statement(pool: &PgPool) {
    sqlx::query("DROP INDEX IF EXISTS grants_open")
        .execute(pool)
        .await
        .expect("drop the index");
    sqlx::query("CREATE UNIQUE INDEX grants_open ON grants (account) WHERE id > 0")
        .execute(pool)
        .await
        .expect("create the partial unique index");
    assert_eq!(
        qbrs::insert::insert(grants::Table)
            .values(grant("ada", "https://fourth"))
            .on_conflict_do_update(
                qbrs::insert::partial_index(grants::account, grants::id.gt(0i64)),
                take_the_url_unless_one_is_held(),
            )
            .execute(pool)
            .await
            .expect("both clauses in one statement"),
        0
    );
}

async fn a_multi_row_upsert_counts_rows_written_rather_than_the_branch_each_took(pool: &PgPool) {
    let one_insert_beside_one_refused_update = qbrs::insert::insert(grants::Table)
        .values(grant("ada", "https://fifth"))
        .values(grant("grace", "https://grace"))
        .on_conflict_do_update(grants::account, take_the_url_unless_one_is_held())
        .execute(pool)
        .await
        .expect("one insert and one refusal");
    assert_eq!(one_insert_beside_one_refused_update, 1);
}
