//! Every aggregate helper run by a real Postgres. String assertions
//! elsewhere say the SQL looks right; only this says Postgres has a
//! function of that name taking arguments of those types, and returning
//! what the helper claims it decodes to.

mod common;

use qbrs::expr::{avg, count, count_of, max, min, string_agg, sum};
use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

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

/// One group's counting aggregates, in the order they are selected.
type Counts = (i64, i64, i64, i64, Option<i64>, Option<i64>, Option<f64>);

#[tokio::test]
async fn every_aggregate_helper_is_a_function_postgres_has() {
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

    let rows: Vec<Counts> = select((
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
    .load(&pool)
    .await
    .expect("the counting aggregates")
    .into_tuples();
    // `count()` counts rows, `count_of` a column's non-NULL values — which
    // is the different question the all-NULL `note` column answers.
    assert_eq!(
        rows,
        vec![(1, 3, 3, 0, Some(10), Some(2), Some(10.0 / 3.0))]
    );

    let labels: Vec<Option<String>> = select(string_agg(line_items::label, ", "))
        .from(line_items::Table)
        .group_by(line_items::order_id)
        .load(&pool)
        .await
        .expect("string_agg over a text column");
    // `string_agg` has no ORDER BY of its own here, so the group's order is
    // the scan's; the parts are what this checks, not their order.
    let joined = labels.into_iter().next().flatten().expect("one group");
    let mut parts: Vec<&str> = joined.split(", ").collect();
    parts.sort_unstable();
    assert_eq!(parts, vec!["bolt", "nut", "washer"]);

    // Over a column that is NULL in every row, `string_agg` is NULL rather
    // than the empty string — which is why it decodes to an `Option`.
    let notes: Vec<Option<String>> = select(string_agg(line_items::note, ", "))
        .from(line_items::Table)
        .load(&pool)
        .await
        .expect("string_agg over a nullable column");
    assert_eq!(notes, vec![None]);

    let widest: Option<String> = select(max(line_items::label))
        .from(line_items::Table)
        .load_one(&pool)
        .await
        .expect("max over text")
        .expect("one row");
    assert_eq!(widest.as_deref(), Some("washer"));

    common::shutdown(pool, guard).await;
}
