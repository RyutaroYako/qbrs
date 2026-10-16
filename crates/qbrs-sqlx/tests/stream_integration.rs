mod common;

use qbrs::prelude::*;
// Importing `futures::StreamExt` here would hide whether the prelude alone
// gives a caller the `next()` a stream is read with.
use qbrs_sqlx::prelude::*;
use sqlx::PgPool;

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
async fn a_select_a_union_and_an_update_returning_each_stream_their_rows() {
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

    a_batching_consumer_streams_the_filtered_rows_in_order_and_load_collects_the_same(&pool).await;
    a_union_streams_its_rows_like_a_select(&pool).await;
    an_update_returning_streams_its_rows_like_a_select(&pool).await;

    common::shutdown(pool, guard).await;
}

async fn a_batching_consumer_streams_the_filtered_rows_in_order_and_load_collects_the_same(
    pool: &PgPool,
) {
    let query = select((readings_stream::label, readings_stream::value))
        .from(readings_stream::Table)
        .filter(readings_stream::value.gte(100i64))
        .order_by(readings_stream::value.desc());

    let mut rows = query.stream(pool).expect("open the stream");
    let mut batch = Vec::new();
    let mut batches = 0usize;
    let mut streamed: Vec<(String, i64)> = Vec::new();
    while let Some(row) = rows.next().await {
        batch.push(row.expect("decode a streamed row").into_tuple());
        if batch.len() == 100 {
            batches += 1;
            streamed.append(&mut batch);
        }
    }
    drop(rows);
    streamed.append(&mut batch);
    assert_eq!(batches, 4);
    let expected: Vec<(String, i64)> = (100..500i64)
        .rev()
        .map(|n| (format!("row-{n}"), n))
        .collect();
    assert_eq!(streamed, expected);

    let collected: Vec<(String, i64)> = query
        .load(pool)
        .await
        .expect("collect the same query")
        .into_tuples();
    assert_eq!(collected, streamed);
}

async fn an_update_returning_streams_its_rows_like_a_select(pool: &PgPool) {
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
        .stream(pool)
        .expect("open the returning stream");
    let mut bumped_values = Vec::new();
    while let Some(value) = bumped.next().await {
        bumped_values.push(value.expect("decode a returned row"));
    }
    drop(bumped);
    bumped_values.sort_unstable();
    assert_eq!(bumped_values, vec![0i64, 1, 2]);
}

async fn a_union_streams_its_rows_like_a_select(pool: &PgPool) {
    let low = select((readings_stream::value,))
        .from(readings_stream::Table)
        .filter(readings_stream::value.lt(2i64));
    let high = select((readings_stream::value,))
        .from(readings_stream::Table)
        .filter(readings_stream::value.gte(498i64));
    let mut both = low
        .union(&high)
        .stream(pool)
        .expect("open the union stream");
    let mut values = Vec::new();
    while let Some(row) = both.next().await {
        values.push(*row.expect("decode a union row").get(readings_stream::value));
    }
    drop(both);
    values.sort_unstable();
    assert_eq!(values, vec![0, 1, 498, 499]);
}

#[tokio::test]
async fn a_stream_dropped_early_gives_the_only_connection_of_its_pool_back() {
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

    let all = query
        .load(&one_at_a_time)
        .await
        .expect("the pool's one connection is back");
    assert_eq!(all.len(), 50);

    one_at_a_time.close().await;
    common::shutdown(pool, guard).await;
}
