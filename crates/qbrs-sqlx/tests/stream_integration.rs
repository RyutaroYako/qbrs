//! `.stream(..)` against a real Postgres: the export that writes as it
//! reads. Only a database says the rows arrive one at a time and decode to
//! what `.load(..)` would have collected.

mod common;

use futures_util::StreamExt as _;
use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "readings_stream")]
#[allow(dead_code)]
struct ReadingsStream {
    #[column(primary_key, generated)]
    id: i64,
    label: String,
    value: i64,
}

#[tokio::test]
async fn a_stream_yields_the_rows_load_would_have_collected() {
    let (pool, guard) = common::test_pool("qbrs_stream").await;

    sqlx::query("DROP TABLE IF EXISTS readings_stream")
        .execute(&pool)
        .await
        .expect("drop readings_stream");
    sqlx::query(
        "CREATE TABLE readings_stream (
            id BIGSERIAL PRIMARY KEY,
            label TEXT NOT NULL,
            value BIGINT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("create readings_stream");

    qbrs::insert::insert(readings_stream::Table)
        .values_all((0..500i64).map(|n| {
            ReadingsStreamInsert::builder()
                .label(format!("row-{n}"))
                .value(n)
                .build()
        }))
        .expect("five hundred rows")
        .execute(&pool)
        .await
        .expect("seed the readings");

    let query = select((readings_stream::label, readings_stream::value))
        .from(readings_stream::Table)
        .filter(readings_stream::value.gte(100i64))
        .order_by(readings_stream::value.asc());

    // The shape the issue was stuck on: consume in batches, holding one
    // batch rather than the whole result.
    let mut rows = query.stream(&pool).expect("open the stream");
    let mut batch = Vec::new();
    let mut batches = 0usize;
    let mut seen = 0usize;
    let mut first = None;
    while let Some(row) = rows.next().await {
        let row = row.expect("decode a streamed row");
        if first.is_none() {
            first = Some(*row.get(readings_stream::value));
        }
        batch.push(row);
        seen += 1;
        if batch.len() == 100 {
            batches += 1;
            batch.clear();
        }
    }
    drop(rows);
    assert_eq!(seen, 400);
    assert_eq!(batches, 4);
    assert_eq!(first, Some(100));

    // Same query, same values: streaming and collecting differ in when the
    // rows arrive, not in what they are.
    let collected = query.load(&pool).await.expect("collect the same query");
    assert_eq!(collected.len(), seen);
    assert_eq!(*collected[0].get(readings_stream::label), "row-100");

    // A `RETURNING` streams too — the terminal is cut by what a statement
    // produces, not by which builder produced it.
    let mut bumped = qbrs::update::update(readings_stream::Table)
        .set(
            qbrs::update::Assignments::from_row(ReadingsStreamUpdate {
                label: Some("bumped".to_string()),
                ..Default::default()
            })
            .expect("label is set"),
        )
        .filter(readings_stream::value.lt(3i64))
        .returning(readings_stream::value)
        .stream(&pool)
        .expect("open the returning stream");
    let mut bumped_values = Vec::new();
    while let Some(value) = bumped.next().await {
        bumped_values.push(value.expect("decode a returned row"));
    }
    bumped_values.sort_unstable();
    assert_eq!(bumped_values, vec![0i64, 1, 2]);
    drop(bumped);

    common::shutdown(pool, guard).await;
}
