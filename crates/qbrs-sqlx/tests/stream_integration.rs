//! `.stream(..)` against a real Postgres: the export that writes as it
//! reads. Only a database says the rows arrive one at a time and decode to
//! what `.load(..)` would have collected.

mod common;

use qbrs::prelude::*;
// `StreamExt::next` comes from the same prelude the terminal does, so a
// caller needs no `futures` dependency of their own.
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "readings_stream_drop")]
#[allow(dead_code)]
struct ReadingsStreamDrop {
    #[column(primary_key, generated)]
    id: i64,
    label: String,
    value: i64,
}

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

    // A `RETURNING` streams too: the terminal is cut by what a statement
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

/// A stream holds its connection until it ends. Dropped early, the
/// connection goes back. Only a pool with one connection in it says so.
#[tokio::test]
async fn a_stream_dropped_early_gives_its_connection_back() {
    let (pool, guard) = common::test_pool("qbrs_stream_drop").await;
    let one_at_a_time = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .expect("a pool of one");

    sqlx::query("DROP TABLE IF EXISTS readings_stream_drop")
        .execute(&one_at_a_time)
        .await
        .expect("drop the table");
    sqlx::query(
        "CREATE TABLE readings_stream_drop (
            id BIGSERIAL PRIMARY KEY,
            label TEXT NOT NULL,
            value BIGINT NOT NULL
        )",
    )
    .execute(&one_at_a_time)
    .await
    .expect("create the table");
    qbrs::insert::insert(readings_stream_drop::Table)
        .values_all((0..50i64).map(|n| {
            ReadingsStreamDropInsert::builder()
                .label(format!("row-{n}"))
                .value(n)
                .build()
        }))
        .expect("fifty rows")
        .execute(&one_at_a_time)
        .await
        .expect("seed the rows");

    let query = select(readings_stream_drop::value)
        .from(readings_stream_drop::Table)
        .order_by(readings_stream_drop::value.asc());

    let mut rows = query.stream(&one_at_a_time).expect("open the stream");
    let first = rows.next().await.expect("a row").expect("decode it");
    assert_eq!(first, 0);
    drop(rows);

    // The only connection there is has to be free again.
    let all = query
        .load(&one_at_a_time)
        .await
        .expect("the pool's one connection is back");
    assert_eq!(all.len(), 50);

    // A `UNION` streams for the same reason a `SELECT` does: the terminal
    // is cut by what a statement produces.
    let low = select((readings_stream_drop::value,))
        .from(readings_stream_drop::Table)
        .filter(readings_stream_drop::value.lt(2i64));
    let high = select((readings_stream_drop::value,))
        .from(readings_stream_drop::Table)
        .filter(readings_stream_drop::value.gte(48i64));
    let mut both = low
        .union(&high)
        .stream(&one_at_a_time)
        .expect("open the union stream");
    let mut values = Vec::new();
    while let Some(row) = both.next().await {
        values.push(
            *row.expect("decode a union row")
                .get(readings_stream_drop::value),
        );
    }
    drop(both);
    values.sort_unstable();
    assert_eq!(values, vec![0, 1, 48, 49]);

    one_at_a_time.close().await;
    common::shutdown(pool, guard).await;
}
