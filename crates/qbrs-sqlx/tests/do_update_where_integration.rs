//! `ON CONFLICT .. DO UPDATE SET .. WHERE ..` against a real Postgres.
//!
//! The clause exists for what it does to `rows_affected()`: a conflicting
//! row the condition rejects is not touched *and not counted*, so a caller
//! can upsert and read "did this write anything" off the count. An
//! unconditional `DO UPDATE` always reports 1, which is what makes the
//! `CASE WHEN` no-op workaround useless for the same question.

mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "grants")]
#[allow(dead_code)]
struct Grants {
    #[column(primary_key, generated)]
    id: i64,
    account: String,
    authorization_url: Option<String>,
}

#[tokio::test]
async fn a_conditional_do_update_leaves_the_rows_it_rejects_uncounted() {
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

    // "Granted once": take the URL if the row has none, and report having
    // written nothing if it already does.
    let grant_once = |url: &'static str| {
        qbrs::insert::insert(grants::Table)
            .values(
                GrantsInsert::builder()
                    .account("ada")
                    .authorization_url(url)
                    .build(),
            )
            .on_conflict_do_update(
                grants::account,
                ConflictUpdate::set_to(
                    grants::authorization_url,
                    excluded(grants::authorization_url),
                )
                .filter(grants::authorization_url.is_null()),
            )
    };

    assert_eq!(
        grant_once("https://first")
            .execute(&pool)
            .await
            .expect("the first grant"),
        1
    );
    // The row now has a URL, so the condition rejects it: nothing written,
    // and — the whole point — nothing counted either.
    assert_eq!(
        grant_once("https://second")
            .execute(&pool)
            .await
            .expect("the second grant"),
        0
    );

    let held: Option<String> = select(grants::authorization_url)
        .from(grants::Table)
        .load_one(&pool)
        .await
        .expect("read the row back")
        .expect("one row");
    assert_eq!(held, Some("https://first".to_string()));

    // Clearing it opens the grant again, so the condition is read per
    // statement rather than decided once.
    qbrs::update::update(grants::Table)
        .set_to(grants::authorization_url, qbrs::expr::null())
        .execute(&pool)
        .await
        .expect("clear the url");
    assert_eq!(
        grant_once("https://third")
            .execute(&pool)
            .await
            .expect("the third grant"),
        1
    );

    common::shutdown(pool, guard).await;
}
