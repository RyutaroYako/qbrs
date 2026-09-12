//! `ON CONFLICT DO UPDATE SET .. excluded.column ..` against a real
//! Postgres.
//!
//! Passing the same Rust value to both halves of an upsert stands in for
//! the proposed row only while every inserted value is a literal the caller
//! already holds. What it cannot say is a column the *database* filled —
//! a default, a sequence, `now()` — or an assignment that reads the
//! conflicting row and the proposed one together. Both are what this asks
//! Postgres about.

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
async fn an_upsert_reads_the_row_the_insert_proposed() {
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

    // `hits = hits + EXCLUDED.hits` and `label = EXCLUDED.label`: the
    // first reads both rows at once, the second takes a value only the
    // server has, since the insert left the column to its default.
    let upsert = |hits: i64| {
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

    // The row starts with a label the caller supplied, so what the update
    // branch leaves behind says which of the two rows `excluded.label` read.
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
    assert_eq!(upsert(4).execute(&pool).await.expect("the upsert"), 1);
    assert_eq!(upsert(5).execute(&pool).await.expect("once more"), 1);

    let row: (i64, String) = select((visit_counts::hits, visit_counts::label))
        .from(visit_counts::Table)
        .load_one(&pool)
        .await
        .expect("read the row back")
        .expect("one row")
        .into_tuple();
    // Accumulated rather than overwritten, which the value-in-both-halves
    // workaround renders as `hits = $2` and cannot.
    assert_eq!(row.0, 12);
    // The default the server filled reached the update branch, having
    // never passed through the caller's hands.
    assert_eq!(row.1, "from the schema");

    common::shutdown(pool, guard).await;
}
