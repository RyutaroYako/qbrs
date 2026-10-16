mod common;

use qbrs::insert::partial_index;
use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;
use sqlx::PgPool;

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

fn core_member(handle: &'static str) -> MembershipsInsert {
    MembershipsInsert::builder()
        .team("core")
        .handle(handle)
        .build()
}

fn join_or_close_out_the_current_membership(
    handle: &'static str,
) -> Insert<Postgres, MembershipsInsert> {
    qbrs::insert::insert(memberships::Table)
        .values(core_member(handle))
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
}

#[tokio::test]
async fn a_conflict_target_repeating_the_index_predicate_reaches_a_partial_unique_index() {
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
    sqlx::query(
        "CREATE UNIQUE INDEX memberships_current
            ON memberships (team, handle) WHERE left_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("create the partial unique index");

    assert_eq!(
        join_or_close_out_the_current_membership("ada")
            .execute(&pool)
            .await
            .expect("first insert"),
        1
    );

    a_conflict_on_the_partial_index_takes_the_update_branch(&pool).await;
    a_row_the_index_no_longer_covers_lets_its_key_insert_again(&pool).await;
    a_target_predicate_implying_the_index_predicate_still_picks_the_index(&pool).await;
    a_target_of_bare_columns_is_refused_rather_than_matching_the_partial_index(&pool).await;

    common::shutdown(pool, guard).await;
}

async fn a_conflict_on_the_partial_index_takes_the_update_branch(pool: &PgPool) {
    assert_eq!(
        join_or_close_out_the_current_membership("ada")
            .execute(pool)
            .await
            .expect("the upsert"),
        1
    );

    let rows: Vec<(String, Option<i64>)> = select((memberships::handle, memberships::left_at))
        .from(memberships::Table)
        .load(pool)
        .await
        .expect("read the rows back")
        .into_tuples();
    assert_eq!(rows, vec![("ada".to_string(), Some(1))]);
}

async fn a_row_the_index_no_longer_covers_lets_its_key_insert_again(pool: &PgPool) {
    assert_eq!(
        join_or_close_out_the_current_membership("ada")
            .execute(pool)
            .await
            .expect("reuse the key"),
        1
    );
    let rows: i64 = select(qbrs::expr::count())
        .from(memberships::Table)
        .load_one(pool)
        .await
        .expect("count the rows")
        .expect("one row");
    assert_eq!(rows, 2);
}

async fn a_target_predicate_implying_the_index_predicate_still_picks_the_index(pool: &PgPool) {
    let inserted = qbrs::insert::insert(memberships::Table)
        .values(core_member("ada"))
        .on_conflict_do_nothing(partial_index(
            (memberships::team, memberships::handle),
            memberships::left_at
                .is_null()
                .and(memberships::team.eq("core")),
        ))
        .returning(memberships::id)
        .load(pool)
        .await
        .expect("a predicate that implies the index's own");
    assert!(inserted.is_empty(), "the current row is the conflict");
}

async fn a_target_of_bare_columns_is_refused_rather_than_matching_the_partial_index(pool: &PgPool) {
    let unmatched = qbrs::insert::insert(memberships::Table)
        .values(core_member("ada"))
        .on_conflict_do_nothing((memberships::team, memberships::handle))
        .execute(pool)
        .await;
    assert!(
        unmatched.is_err(),
        "bare columns must not reach the partial index"
    );
}
