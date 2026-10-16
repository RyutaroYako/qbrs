mod common;

use qbrs::expr::{avg, count, count_of, max, min, string_agg, sum};
use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;
use sqlx::PgPool;

#[derive(Table)]
#[table(name = "line_items")]
#[allow(dead_code)]
struct LineItems {
    #[column(primary_key, generated)]
    id: i64,
    order_id: i64,
    label: String,
    note: Option<String>,
    quantity: i64,
}

type CountingAggregatesRow = (i64, i64, i64, i64, Option<i64>, Option<i64>, Option<f64>);

#[tokio::test]
async fn every_aggregate_helper_is_a_function_postgres_has_and_decodes_as_declared() {
    let (pool, guard) = common::test_pool("qbrs_aggregates").await;

    sqlx::query("DROP TABLE IF EXISTS line_items")
        .execute(&pool)
        .await
        .expect("drop line_items");
    sqlx::query(
        "CREATE TABLE line_items (
            id BIGSERIAL PRIMARY KEY,
            order_id BIGINT NOT NULL,
            label TEXT NOT NULL,
            note TEXT,
            quantity BIGINT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("create line_items");

    qbrs::insert::insert(line_items::Table)
        .values_all(
            [("bolt", 2i64), ("nut", 3), ("washer", 5)].map(|(label, quantity)| {
                LineItemsInsert::builder()
                    .order_id(1i64)
                    .label(label)
                    .quantity(quantity)
                    .build()
            }),
        )
        .expect("three line items")
        .execute(&pool)
        .await
        .expect("insert line items");

    count_counts_rows_while_count_of_counts_a_columns_non_null_values(&pool).await;
    a_string_agg_separator_is_bound_as_data_rather_than_spliced_as_sql(&pool).await;
    string_agg_joins_a_text_column_with_its_separator(&pool).await;
    string_agg_over_a_column_null_in_every_row_is_null_rather_than_empty(&pool).await;
    max_over_a_text_column_decodes_to_the_greatest_string(&pool).await;

    common::shutdown(pool, guard).await;
}

async fn count_counts_rows_while_count_of_counts_a_columns_non_null_values(pool: &PgPool) {
    let rows: Vec<CountingAggregatesRow> = select((
        line_items::order_id,
        count(),
        count_of(line_items::quantity),
        count_of(line_items::note),
        sum(line_items::quantity),
        min(line_items::quantity),
        avg(line_items::quantity),
    ))
    .from(line_items::Table)
    .group_by(line_items::order_id)
    .load(pool)
    .await
    .expect("the counting aggregates")
    .into_tuples();
    assert_eq!(
        rows,
        vec![(1, 3, 3, 0, Some(10), Some(2), Some(10.0 / 3.0))]
    );
}

async fn a_string_agg_separator_is_bound_as_data_rather_than_spliced_as_sql(pool: &PgPool) {
    // A plain separator would come back the same whether it was bound or
    // spliced. This one is SQL syntax, including under
    // `standard_conforming_strings = off`, so only binding returns it intact.
    let separator = r"') FROM x -- \";
    let injected: Option<String> = select(string_agg(line_items::label, separator))
        .from(line_items::Table)
        .load_one(pool)
        .await
        .expect("a separator that is only ever data")
        .expect("one row");
    let joined = injected.expect("three labels run together");
    let mut parts: Vec<&str> = joined.split(separator).collect();
    parts.sort_unstable();
    assert_eq!(parts, vec!["bolt", "nut", "washer"]);
}

async fn string_agg_joins_a_text_column_with_its_separator(pool: &PgPool) {
    let labels: Vec<Option<String>> = select(string_agg(line_items::label, ", "))
        .from(line_items::Table)
        .group_by(line_items::order_id)
        .load(pool)
        .await
        .expect("string_agg over a text column");
    let joined = labels.into_iter().next().flatten().expect("one group");
    // Comparing the joined string as a whole would depend on scan order,
    // since this `string_agg` has no `ORDER BY` of its own.
    let mut parts: Vec<&str> = joined.split(", ").collect();
    parts.sort_unstable();
    assert_eq!(parts, vec!["bolt", "nut", "washer"]);
}

async fn string_agg_over_a_column_null_in_every_row_is_null_rather_than_empty(pool: &PgPool) {
    let notes: Vec<Option<String>> = select(string_agg(line_items::note, ", "))
        .from(line_items::Table)
        .load(pool)
        .await
        .expect("string_agg over a nullable column");
    assert_eq!(notes, vec![None]);
}

async fn max_over_a_text_column_decodes_to_the_greatest_string(pool: &PgPool) {
    let widest: Option<String> = select(max(line_items::label))
        .from(line_items::Table)
        .load_one(pool)
        .await
        .expect("max over text")
        .expect("one row");
    assert_eq!(widest.as_deref(), Some("washer"));
}
