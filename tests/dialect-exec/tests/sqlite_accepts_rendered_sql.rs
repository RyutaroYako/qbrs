//! Every statement qbrs renders for `Sqlite` is handed to a real SQLite and
//! executed. String assertions elsewhere say the SQL looks right; only this
//! says SQLite agrees.

use qbrs::cte::with;
use qbrs::expr::{Expr, Value, all_of, any_of, avg, count, count_of, max, min, string_agg, sum};
use qbrs::insert::partial_index;
use qbrs::prelude::*;
use qbrs::sql;
use sqlx::{Row, SqlitePool};

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
}

#[derive(Table)]
#[table(name = "archived_orders")]
#[allow(dead_code)]
struct ArchivedOrders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

#[derive(Table)]
#[table(name = "orders")]
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

with! {
    struct big_orders { user_id: BigInt, total: Nullable<BigInt> }
}

const PARTIAL_UNIQUE_INDEX_FOR_CONFLICT_TARGETS: &str =
    "CREATE UNIQUE INDEX users_named ON users (display_name) WHERE display_name IS NOT NULL";

async fn schema() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    for ddl in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE, display_name TEXT)",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, total INTEGER NOT NULL)",
        "CREATE TABLE archived_orders (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, total INTEGER NOT NULL)",
        PARTIAL_UNIQUE_INDEX_FOR_CONFLICT_TARGETS,
    ] {
        sqlx::query(sqlx::AssertSqlSafe(ddl))
            .execute(&pool)
            .await
            .expect("create schema");
    }
    pool
}

async fn insert_named_ada_with_orders_of_100_and_2000_and_unnamed_dan_with_30(
    pool: &SqlitePool,
) -> Vec<sqlx::sqlite::SqliteRow> {
    run(
        pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("ada@example.com")
                    .display_name("Ada")
                    .build(),
            )
            .values(UsersInsert::builder().email("dan@example.com").build())
            .to_sql(Sqlite),
    )
    .await;
    run(
        pool,
        insert(orders::Table)
            .values(OrdersInsert::builder().user_id(1).total(100).build())
            .values(OrdersInsert::builder().user_id(1).total(2000).build())
            .values(OrdersInsert::builder().user_id(2).total(30).build())
            .returning(orders::id)
            .to_sql(Sqlite),
    )
    .await
}

async fn seeded() -> SqlitePool {
    let pool = schema().await;
    insert_named_ada_with_orders_of_100_and_2000_and_unnamed_dan_with_30(&pool).await;
    pool
}

async fn run(
    pool: &SqlitePool,
    (sql, params): (String, Vec<Value>),
) -> Vec<sqlx::sqlite::SqliteRow> {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.clone()));
    for p in params {
        q = match p {
            Value::I32(v) => q.bind(v),
            Value::I64(v) => q.bind(v),
            Value::F64(v) => q.bind(v),
            Value::Text(v) => q.bind(v),
            Value::Bool(v) => q.bind(v),
            Value::Bytes(v) => q.bind(v),
            Value::NullI32 => q.bind(None::<i32>),
            Value::NullI64 => q.bind(None::<i64>),
            Value::NullF64 => q.bind(None::<f64>),
            Value::NullText => q.bind(None::<String>),
            Value::NullBool => q.bind(None::<bool>),
            Value::NullBytes => q.bind(None::<Vec<u8>>),
            Value::Placeholder(name) => panic!("unresolved placeholder {name}"),
            // Not removable: the feature-gated variants exist whenever another
            // crate in the build enables those features, and this battery
            // binds none of them.
            #[allow(unreachable_patterns)]
            other => panic!("no SQLite binding for {other:?}"),
        };
    }
    q.fetch_all(pool)
        .await
        .unwrap_or_else(|e| panic!("sqlite rejected `{sql}`: {e}"))
}

async fn display_name_of(pool: &SqlitePool, email: &str) -> Option<String> {
    let rows = run(
        pool,
        select(users::display_name)
            .from(users::Table)
            .filter(users::email.eq(email.to_string()))
            .to_sql(Sqlite),
    )
    .await;
    rows[0].get(0)
}

fn strings(rows: &[sqlx::sqlite::SqliteRow]) -> Vec<String> {
    rows.iter().map(|row| row.get(0)).collect()
}

async fn single_i64(pool: &SqlitePool, statement: (String, Vec<Value>)) -> i64 {
    run(pool, statement).await[0].get::<i64, _>(0)
}

#[tokio::test]
async fn a_multi_row_insert_returns_one_id_per_row() {
    let pool = schema().await;
    let rows = insert_named_ada_with_orders_of_100_and_2000_and_unnamed_dan_with_30(&pool).await;
    let ids: Vec<i64> = rows.iter().map(|row| row.get(0)).collect();
    assert_eq!(ids, [1, 2, 3]);
}

#[tokio::test]
async fn on_conflict_do_update_assigns_the_update_row_to_the_conflicting_row() {
    let pool = seeded().await;
    run(
        &pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("ada@example.com")
                    .display_name("Ada L.")
                    .build(),
            )
            .on_conflict_do_update(
                users::email,
                Assignments::from_row(UsersUpdate {
                    display_name: Some(Some("Ada Lovelace".into())),
                    ..Default::default()
                })
                .expect("display_name is set"),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(
        display_name_of(&pool, "ada@example.com").await.as_deref(),
        Some("Ada Lovelace")
    );
}

#[tokio::test]
async fn on_conflict_do_update_reads_the_proposed_row_through_excluded() {
    let pool = seeded().await;
    run(
        &pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("ada@example.com")
                    .display_name("Ada A.")
                    .build(),
            )
            .on_conflict_do_update(
                users::email,
                ConflictUpdate::set_to(users::display_name, excluded(users::display_name)),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(
        display_name_of(&pool, "ada@example.com").await.as_deref(),
        Some("Ada A.")
    );
}

#[tokio::test]
async fn a_do_update_whose_filter_rejects_the_conflicting_row_leaves_it_unchanged() {
    let pool = seeded().await;
    run(
        &pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("ada@example.com")
                    .display_name("Ada B.")
                    .build(),
            )
            .on_conflict_do_update(
                users::email,
                ConflictUpdate::set_to(users::display_name, excluded(users::display_name))
                    .filter(users::display_name.is_null()),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(
        display_name_of(&pool, "ada@example.com").await.as_deref(),
        Some("Ada")
    );
}

#[tokio::test]
async fn on_conflict_do_nothing_skips_the_conflicting_row() {
    let pool = seeded().await;
    run(
        &pool,
        insert(users::Table)
            .values(UsersInsert::builder().email("ada@example.com").build())
            .on_conflict_do_nothing(users::email)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(
        display_name_of(&pool, "ada@example.com").await.as_deref(),
        Some("Ada")
    );
}

#[tokio::test]
async fn sqlite_infers_a_partial_unique_index_from_a_conflict_target_carrying_its_predicate() {
    let pool = schema().await;
    let seeded = run(
        &pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("partial-seed@example.com")
                    .display_name("Partial Seed")
                    .build(),
            )
            .returning(users::id)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(seeded.len(), 1);

    let dropped = run(
        &pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("partial-again@example.com")
                    .display_name("Partial Seed")
                    .build(),
            )
            .on_conflict_do_nothing(partial_index(
                users::display_name,
                users::display_name.is_not_null(),
            ))
            .returning(users::id)
            .to_sql(Sqlite),
    )
    .await;
    assert!(
        dropped.is_empty(),
        "the partial index should have dropped the row"
    );
}

#[tokio::test]
async fn an_update_sets_a_nullable_column_to_null() {
    let pool = seeded().await;
    run(
        &pool,
        update(users::Table)
            .set(
                Assignments::from_row(UsersUpdate {
                    display_name: Some(None),
                    ..Default::default()
                })
                .expect("display_name is set"),
            )
            .filter(users::email.eq("ada@example.com"))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(display_name_of(&pool, "ada@example.com").await, None);
}

#[tokio::test]
async fn an_update_assigns_an_expression_the_database_computes() {
    let pool = seeded().await;
    run(
        &pool,
        update(users::Table)
            .set_to(users::display_name, sql!(Text, "upper(?)", users::email))
            .filter(users::id.eq(1i64))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(
        display_name_of(&pool, "ada@example.com").await.as_deref(),
        Some("ADA@EXAMPLE.COM")
    );
}

#[tokio::test]
async fn a_left_join_with_order_by_and_limit_returns_each_matching_order() {
    let pool = seeded().await;
    let joined = run(
        &pool,
        select((users::email, orders::total))
            .from(users::Table)
            .left_join(orders::Table, orders::user_id.eq(users::id))
            .filter(users::email.eq("ada@example.com"))
            .order_by(orders::total.desc())
            .limit(10)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(joined.len(), 2);
}

#[tokio::test]
async fn a_right_join_keeps_the_joined_tables_row_that_matches_nothing_with_a_null_left_side() {
    let pool = seeded().await;
    run(
        &pool,
        insert(users::Table)
            .values(UsersInsert::builder().email("eve@example.com").build())
            .to_sql(Sqlite),
    )
    .await;
    let right = run(
        &pool,
        select((users::email, orders::total))
            .from(orders::Table)
            .right_join(users::Table, orders::user_id.eq(users::id))
            .order_by(users::id.asc())
            .order_by(orders::total.asc())
            .to_sql(Sqlite),
    )
    .await;
    let pairs: Vec<(String, Option<i64>)> =
        right.iter().map(|row| (row.get(0), row.get(1))).collect();
    assert_eq!(
        pairs,
        [
            ("ada@example.com".to_string(), Some(100)),
            ("ada@example.com".to_string(), Some(2000)),
            ("dan@example.com".to_string(), Some(30)),
            ("eve@example.com".to_string(), None),
        ]
    );
}

#[tokio::test]
async fn a_full_join_returns_every_matched_pair() {
    let pool = seeded().await;
    let full = run(
        &pool,
        select((users::email, orders::total))
            .from(users::Table)
            .full_join(orders::Table, orders::user_id.eq(users::id))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(full.len(), 3);
}

#[tokio::test]
async fn having_keeps_only_the_groups_past_its_threshold() {
    let pool = seeded().await;
    let grouped = run(
        &pool,
        select((orders::user_id, sum(orders::total), count()))
            .from(orders::Table)
            .group_by(orders::user_id)
            .having(sum(orders::total).gt(50i64))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(grouped.len(), 1, "only Ada's orders sum past 50");
}

#[tokio::test]
async fn a_bare_offset_runs_because_the_dialect_supplies_the_limit_sqlite_requires() {
    let pool = seeded().await;
    let offset_only = run(
        &pool,
        select(users::email)
            .from(users::Table)
            .order_by(users::id.asc())
            .offset(1)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(offset_only.len(), 1);
}

#[tokio::test]
async fn a_union_branch_applies_its_own_limit_rather_than_the_compounds() {
    let pool = seeded().await;
    let paged_union = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .order_by(users::id.asc())
            .limit(1)
            .union(&select((users::email,)).from(users::Table))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(paged_union.len(), 2);
}

#[tokio::test]
async fn a_union_branch_ordered_on_its_own_still_returns_both_branches_rows() {
    let pool = seeded().await;
    let ordered_union = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .order_by(users::id.asc())
            .union(&select((users::email,)).from(users::Table))
            .to_sql(Sqlite),
    )
    .await;
    let mut emails = strings(&ordered_union);
    emails.sort_unstable();
    assert_eq!(emails, ["ada@example.com", "dan@example.com"]);
}

#[tokio::test]
async fn a_union_branch_with_its_own_with_clause_returns_the_cte_rows_beside_the_other_branch() {
    let pool = seeded().await;
    let cte_branch = run(
        &pool,
        select((big_orders::user_id,))
            .from(users::Table)
            .inner_join(
                with(
                    big_orders::Table,
                    &select((orders::user_id, sum(orders::total)))
                        .from(orders::Table)
                        .group_by(orders::user_id),
                ),
                big_orders::user_id.eq(users::id),
            )
            .filter(users::id.eq(1))
            .union(
                &select((orders::user_id,))
                    .from(orders::Table)
                    .filter(orders::total.lt(50)),
            )
            .to_sql(Sqlite),
    )
    .await;
    let mut user_ids: Vec<i64> = cte_branch.iter().map(|row| row.get(0)).collect();
    user_ids.sort_unstable();
    assert_eq!(user_ids, [1, 2]);
}

#[tokio::test]
async fn a_union_returns_the_rows_of_both_filtered_branches() {
    let pool = seeded().await;
    let union = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(users::email.eq("ada@example.com"))
            .union(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.eq("dan@example.com")),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(union.len(), 2);
}

#[tokio::test]
async fn intersect_keeps_only_the_rows_both_branches_return() {
    let pool = seeded().await;
    let intersected = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .intersect(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(strings(&intersected), ["ada@example.com"]);
}

#[tokio::test]
async fn except_drops_the_rows_the_second_branch_returns() {
    let pool = seeded().await;
    let excepted = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .except(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(strings(&excepted), ["dan@example.com"]);
}

#[tokio::test]
async fn a_union_then_intersect_chain_applies_the_union_first_as_the_builder_folds_it() {
    let pool = seeded().await;
    let mixed = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .union(
                &select((users::email,))
                    .from::<Sqlite, _>(users::Table)
                    .filter(users::email.like("dan%")),
            )
            .intersect(
                &select((users::email,))
                    .from::<Sqlite, _>(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(mixed.len(), 1);
}

#[tokio::test]
async fn a_set_operation_orders_by_ordinal_and_pages_its_combined_rows() {
    let pool = seeded().await;
    let paged_union = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .union_all(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .order_by_column(users::email, SortDir::Desc)
            .limit(2)
            .offset(1)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(
        strings(&paged_union),
        ["ada@example.com", "ada@example.com"]
    );
}

#[tokio::test]
async fn a_cte_joins_into_the_outer_query_like_a_table() {
    let pool = seeded().await;
    let totals = select((orders::user_id, sum(orders::total)))
        .from(orders::Table)
        .group_by(orders::user_id);
    let cte = run(
        &pool,
        select((users::email, big_orders::total))
            .from(users::Table)
            .inner_join(
                with(big_orders::Table, &totals),
                big_orders::user_id.eq(users::id),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(cte.len(), 2);
}

#[tokio::test]
async fn a_correlated_exists_keeps_only_the_users_whose_own_orders_match_the_subquery() {
    let pool = seeded().await;
    let exists_q = select(users::email).from(users::Table);
    let correlated = exists_q
        .correlated(orders::Table, orders::id)
        .filter(orders::user_id.eq(users::id))
        .filter(orders::total.gt(1000i64))
        .exists();
    let rows = run(&pool, exists_q.filter(correlated).to_sql(Sqlite)).await;
    assert_eq!(strings(&rows), ["ada@example.com"]);
}

#[tokio::test]
async fn not_exists_keeps_every_row_when_the_subquery_finds_none() {
    let pool = seeded().await;
    let rows = run(
        &pool,
        select(users::email)
            .from(users::Table)
            .filter(
                select((orders::id,))
                    .from(orders::Table)
                    .filter(orders::total.lt(0i64))
                    .not_exists(),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn in_subquery_keeps_only_the_users_the_subquery_selects() {
    let pool = seeded().await;
    let big_spenders = select(orders::user_id)
        .from(orders::Table)
        .filter(orders::total.gt(50i64));
    let contains_rows = run(
        &pool,
        select(users::email)
            .from(users::Table)
            .filter(big_spenders.contains(users::id))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(strings(&contains_rows), ["ada@example.com"]);
}

#[tokio::test]
async fn not_in_subquery_keeps_only_the_users_the_subquery_does_not_select() {
    let pool = seeded().await;
    let big_spenders = select(orders::user_id)
        .from(orders::Table)
        .filter(orders::total.gt(50i64));
    let not_contains_rows = run(
        &pool,
        select(users::email)
            .from(users::Table)
            .filter(big_spenders.not_contains(users::id))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(strings(&not_contains_rows), ["dan@example.com"]);
}

#[tokio::test]
async fn a_labelled_row_number_over_a_partition_yields_one_row_per_order() {
    let pool = seeded().await;
    qbrs::label!(rank_in_user);
    let ranked = run(
        &pool,
        select((
            orders::user_id,
            row_number()
                .over(
                    window()
                        .partition_by(orders::user_id)
                        .order_by(orders::total.desc()),
                )
                .label(label::rank_in_user),
        ))
        .from(orders::Table)
        .to_sql(Sqlite),
    )
    .await;
    assert_eq!(ranked.len(), 3);
}

#[tokio::test]
async fn a_window_comparison_in_order_by_sorts_the_top_ranked_row_first() {
    let pool = seeded().await;
    let sorted = run(
        &pool,
        select((orders::total,))
            .from(orders::Table)
            .order_by(
                row_number()
                    .over(window().order_by(orders::total.desc()))
                    .gt(1i64)
                    .asc(),
            )
            .to_sql(Sqlite),
    )
    .await;
    let totals: Vec<i64> = sorted.iter().map(|row| row.get(0)).collect();
    assert_eq!(totals[0], 2000);
    assert_eq!(totals.len(), 3);
}

#[tokio::test]
async fn a_sql_fragment_filters_by_the_value_bound_into_its_slot() {
    let pool = seeded().await;
    let raw = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(sql!(Bool, "email = ?", "dan@example.com"))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(strings(&raw), ["dan@example.com"]);
}

#[tokio::test]
async fn distinct_collapses_the_rows_a_join_repeats() {
    let pool = seeded().await;
    let distinct = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .inner_join(orders::Table, orders::user_id.eq(users::id))
            .distinct()
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(distinct.len(), 2);
}

#[tokio::test]
async fn select_distinct_sorts_by_a_key_taken_from_its_selection() {
    let pool = seeded().await;
    let distinct_sorted = run(
        &pool,
        select((users::email, users::display_name))
            .from(users::Table)
            .distinct()
            .order_by_selected(users::email, SortDir::Desc)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(
        strings(&distinct_sorted),
        ["dan@example.com", "ada@example.com"]
    );
}

#[tokio::test]
async fn values_all_inserts_every_row_in_one_statement() {
    let pool = seeded().await;
    let bulk = run(
        &pool,
        insert(orders::Table)
            .values_all((10..13).map(|n| OrdersInsert::builder().user_id(1).total(n).build()))
            .expect("three rows")
            .returning(orders::id)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(bulk.len(), 3);
}

#[tokio::test]
async fn a_delete_returns_each_removed_row_and_leaves_the_rest() {
    let pool = seeded().await;
    let deleted = run(
        &pool,
        delete(orders::Table)
            .filter(orders::total.lt(50i64))
            .returning(orders::id)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(deleted.len(), 1);

    let remaining = single_i64(&pool, select(count()).from(orders::Table).to_sql(Sqlite)).await;
    assert_eq!(remaining, 2);
}

#[tokio::test]
async fn count_sql_counts_every_matching_row_whatever_the_limit() {
    let pool = seeded().await;
    let total = single_i64(
        &pool,
        select((orders::id,))
            .from(orders::Table)
            .filter(orders::total.gt(0i64))
            .order_by(orders::id.desc())
            .limit(1)
            .count_sql(Sqlite),
    )
    .await;
    assert_eq!(total, 3);
}

#[tokio::test]
async fn count_sql_of_a_grouped_query_counts_its_groups() {
    let pool = seeded().await;
    let grouped_total = single_i64(
        &pool,
        select((orders::user_id, count()))
            .from(orders::Table)
            .group_by(orders::user_id)
            .having_all(vec![predicate(count().gte(1i64))])
            .count_sql(Sqlite),
    )
    .await;
    assert_eq!(grouped_total, 2);
}

#[tokio::test]
async fn count_sql_of_a_set_operation_counts_its_combined_rows() {
    let pool = seeded().await;
    let union_total = single_i64(
        &pool,
        select((users::email,))
            .from(users::Table)
            .union(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .count_sql(Sqlite),
    )
    .await;
    assert_eq!(union_total, 2);
}

#[tokio::test]
async fn an_erased_select_applies_a_limit_given_after_erasure() {
    let pool = seeded().await;
    let erased = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .erase()
            .limit(1)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(erased.len(), 1);
}

#[tokio::test]
async fn conditions_built_from_and_or_not_null_checks_and_empty_folds_keep_the_rows_they_match() {
    let pool = seeded().await;
    let combined = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(
                users::display_name
                    .is_not_null()
                    .and(users::email.ne("nobody@example.com"))
                    .or(users::id.lte(0i64)),
            )
            .filter(!users::display_name.is_null())
            .filter(all_of(Vec::<Expr<Nil, Bool>>::new()))
            .filter(any_of([users::id.gte(1i64)]))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(strings(&combined), ["ada@example.com"]);
}

#[tokio::test]
async fn count_of_a_column_skips_the_nulls_that_count_counts() {
    let pool = seeded().await;
    let counted = run(
        &pool,
        select((count(), count_of(users::display_name)))
            .from(users::Table)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(
        (counted[0].get::<i64, _>(0), counted[0].get::<i64, _>(1)),
        (2, 1)
    );
}

#[tokio::test]
async fn avg_min_and_max_return_the_mean_least_and_greatest_total() {
    let pool = seeded().await;
    let stats = run(
        &pool,
        select((avg(orders::total), min(orders::total), max(orders::total)))
            .from(orders::Table)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(
        (
            stats[0].get::<f64, _>(0),
            stats[0].get::<i64, _>(1),
            stats[0].get::<i64, _>(2)
        ),
        (710.0, 30, 2000)
    );
}

#[tokio::test]
async fn string_agg_keeps_a_separator_that_reads_as_sql_syntax_intact() {
    let pool = seeded().await;
    let separator_resembling_sql_syntax = r"')--";
    let names = run(
        &pool,
        select((string_agg(users::email, separator_resembling_sql_syntax),))
            .from(users::Table)
            .filter(users::email.like("%@example.com"))
            .to_sql(Sqlite),
    )
    .await;
    let emails = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(users::email.like("%@example.com"))
            .order_by(users::email.asc())
            .to_sql(Sqlite),
    )
    .await;
    let expected: Vec<String> = emails.iter().map(|row| row.get(0)).collect();
    let joined: Option<String> = names[0].get(0);
    let joined = joined.expect("the users run together");
    let mut parts: Vec<&str> = joined.split(separator_resembling_sql_syntax).collect();
    parts.sort_unstable();
    assert_eq!(
        parts,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn insert_select_copies_only_the_rows_its_filtered_query_returns() {
    let pool = seeded().await;
    run(
        &pool,
        insert(archived_orders::Table)
            .select(
                &select((orders::user_id, orders::total))
                    .from(orders::Table)
                    .filter(orders::total.gt(50i64)),
            )
            .to_sql(Sqlite),
    )
    .await;
    let archived = run(
        &pool,
        select((archived_orders::total,))
            .from(archived_orders::Table)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(archived.len(), 2);
    assert!(
        archived.iter().all(|row| row.get::<i64, _>(0) > 50),
        "only the orders over 50 should have been copied"
    );
}

#[tokio::test]
async fn a_tuple_select_without_from_binds_its_arguments_positionally() {
    let pool = schema().await;
    let computed = single_i64(
        &pool,
        select((sql!(BigInt, "(? - ?)", 5i64, 3i64),)).to_sql(Sqlite),
    )
    .await;
    assert_eq!(computed, 2);
}

#[tokio::test]
async fn a_bare_select_without_from_binds_its_arguments_positionally() {
    let pool = schema().await;
    let bare = single_i64(
        &pool,
        select(sql!(BigInt, "(? / ?)", 42i64, 6i64)).to_sql(Sqlite),
    )
    .await;
    assert_eq!(bare, 7);
}

#[tokio::test]
async fn is_in_keeps_a_row_whose_value_is_listed() {
    let pool = seeded().await;
    let in_list = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(users::email.is_in(["ada@example.com".to_string()]))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(in_list.len(), 1);
}

#[tokio::test]
async fn an_empty_is_in_list_matches_no_row() {
    let pool = seeded().await;
    let in_list = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(users::id.is_in(Vec::<i64>::new()))
            .to_sql(Sqlite),
    )
    .await;
    assert!(in_list.is_empty());
}

#[tokio::test]
async fn a_fragment_repeated_in_three_clauses_carries_its_binds_once_per_clause() {
    let pool = seeded().await;
    let bucket = sql!(BigInt, "((? / ?) * ?)", orders::total, 1000i64, 1000i64);
    let buckets = run(
        &pool,
        select((bucket.clone(), count()))
            .from(orders::Table)
            .group_by(bucket.clone())
            .order_by(bucket.asc())
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(buckets.len(), 2, "totals of 30 and 100 share bucket 0");
}
