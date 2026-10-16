mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "visit_counts")]
#[allow(dead_code)]
struct VisitCounts {
    #[column(primary_key, generated)]
    id: i64,
    path: String,
    hits: i64,
    #[column(default)]
    label: String,
}

#[tokio::test]
async fn an_upsert_accumulates_from_excluded_and_takes_the_default_the_server_filled() {
    let (pool, guard) = common::test_pool("qbrs_excluded").await;

    sqlx::query("DROP TABLE IF EXISTS visit_counts")
        .execute(&pool)
        .await
        .expect("drop visit_counts");
    sqlx::query(
        "CREATE TABLE visit_counts (
            id BIGSERIAL PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            hits BIGINT NOT NULL,
            label TEXT NOT NULL DEFAULT 'from the schema'
        )",
    )
    .execute(&pool)
    .await
    .expect("create visit_counts");

    let upsert_omitting_label = |hits: i64| {
        qbrs::insert::insert(visit_counts::Table)
            .values(
                VisitCountsInsert::builder()
                    .path("/index")
                    .hits(hits)
                    .build(),
            )
            .on_conflict_do_update(
                visit_counts::path,
                ConflictUpdate::set_to(
                    visit_counts::hits,
                    qbrs::sql!(
                        qbrs::expr::BigInt,
                        "(? + ?)",
                        visit_counts::hits,
                        excluded(visit_counts::hits)
                    ),
                )
                .and_set_to(visit_counts::label, excluded(visit_counts::label)),
            )
    };

    // Seeding with the column's default would make the existing row and the
    // proposed one agree on `label`, hiding which of them `excluded` read.
    assert_eq!(
        qbrs::insert::insert(visit_counts::Table)
            .values(
                VisitCountsInsert::builder()
                    .path("/index")
                    .hits(3)
                    .label("from the caller")
                    .build(),
            )
            .execute(&pool)
            .await
            .expect("first insert"),
        1
    );
    assert_eq!(
        upsert_omitting_label(4)
            .execute(&pool)
            .await
            .expect("the upsert"),
        1
    );
    assert_eq!(
        upsert_omitting_label(5)
            .execute(&pool)
            .await
            .expect("once more"),
        1
    );

    let (hits, label): (i64, String) = select((visit_counts::hits, visit_counts::label))
        .from(visit_counts::Table)
        .load_one(&pool)
        .await
        .expect("read the row back")
        .expect("one row")
        .into_tuple();
    let hits_summed_over_the_insert_and_both_upserts = 3 + 4 + 5;
    assert_eq!(hits, hits_summed_over_the_insert_and_both_upserts);
    assert_eq!(label, "from the schema");

    common::shutdown(pool, guard).await;
}
