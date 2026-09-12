//! `ON CONFLICT (..) WHERE ..` against a real partial unique index.
//!
//! Postgres infers which index a conflict target means, and a target of
//! bare columns matches only an *unfiltered* index over them. Whether
//! repeating the index's predicate reaches a partial one is Postgres's
//! answer to give, not a string assertion's — and the same run says the
//! rows the index does not cover still insert rather than conflicting.

mod common;

use qbrs::insert::partial_index;
use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "memberships")]
#[allow(dead_code)]
struct Memberships {
    #[column(primary_key, generated)]
    id: i64,
    team: String,
    handle: String,
    left_at: Option<i64>,
}

#[tokio::test]
async fn a_conflict_target_reaches_a_partial_unique_index() {
    let (pool, guard) = common::test_pool("qbrs_partial_index").await;

    sqlx::query("DROP TABLE IF EXISTS memberships")
        .execute(&pool)
        .await
        .expect("drop memberships");
    sqlx::query(
        "CREATE TABLE memberships (
            id BIGSERIAL PRIMARY KEY,
            team TEXT NOT NULL,
            handle TEXT NOT NULL,
            left_at BIGINT
        )",
    )
    .execute(&pool)
    .await
    .expect("create memberships");
    // The soft-delete shape: a natural key unique only among the rows that
    // are still current, so it can be reused once one is closed out.
    sqlx::query(
        "CREATE UNIQUE INDEX memberships_current
            ON memberships (team, handle) WHERE left_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("create the partial unique index");

    let upsert = |handle: &'static str| {
        qbrs::insert::insert(memberships::Table)
            .values(
                MembershipsInsert::builder()
                    .team("core")
                    .handle(handle)
                    .build(),
            )
            .on_conflict_do_update(
                partial_index(
                    (memberships::team, memberships::handle),
                    memberships::left_at.is_null(),
                ),
                qbrs::update::Assignments::from_row(MembershipsUpdate {
                    left_at: Some(Some(1)),
                    ..Default::default()
                })
                .expect("left_at is set"),
            )
    };

    assert_eq!(upsert("ada").execute(&pool).await.expect("first insert"), 1);
    // The second one conflicts with the partial index and takes the update
    // branch — which is the whole point: without the predicate Postgres
    // finds no index to infer and rejects the statement outright.
    assert_eq!(upsert("ada").execute(&pool).await.expect("the upsert"), 1);

    let rows: Vec<(String, Option<i64>)> = select((memberships::handle, memberships::left_at))
        .from(memberships::Table)
        .load(&pool)
        .await
        .expect("read the rows back")
        .into_tuples();
    assert_eq!(rows, vec![("ada".to_string(), Some(1))]);

    // That row is now outside the index, so the same natural key inserts
    // again rather than conflicting.
    assert_eq!(
        upsert("ada").execute(&pool).await.expect("reuse the key"),
        1
    );
    let current: i64 = select(qbrs::expr::count())
        .from(memberships::Table)
        .load_one(&pool)
        .await
        .expect("count the rows")
        .expect("one row");
    assert_eq!(current, 2);

    // Matching is implication, not equality: a predicate that says more
    // than the index's still picks it.
    let narrower = qbrs::insert::insert(memberships::Table)
        .values(
            MembershipsInsert::builder()
                .team("core")
                .handle("ada")
                .build(),
        )
        .on_conflict_do_nothing(partial_index(
            (memberships::team, memberships::handle),
            memberships::left_at
                .is_null()
                .and(memberships::team.eq("core")),
        ))
        .returning(memberships::id)
        .load(&pool)
        .await
        .expect("a predicate that implies the index's own");
    assert!(narrower.is_empty(), "the current row is the conflict");

    // One that implies no index is refused rather than quietly matching a
    // different one.
    let unmatched = qbrs::insert::insert(memberships::Table)
        .values(
            MembershipsInsert::builder()
                .team("core")
                .handle("ada")
                .build(),
        )
        .on_conflict_do_nothing((memberships::team, memberships::handle))
        .execute(&pool)
        .await;
    assert!(
        unmatched.is_err(),
        "bare columns must not reach the partial index"
    );

    common::shutdown(pool, guard).await;
}
