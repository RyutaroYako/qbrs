//! `.stream(..)`: the rows one at a time, for a result too large to hold.
//! An export can then write as it reads rather than collecting first.
//! Not a cursor: the server still produces the whole result and the
//! connection is held until the stream ends; what this bounds is the
//! client's memory. The rows arrive as they are read, so the export below
//! flushes them in batches and never holds more than one batch. Streaming and
//! collecting differ in when the rows arrive, not in what they are.
//! Run: `cargo run -p qbrs-examples --example 27_stream`

use qbrs::prelude::*;
use qbrs_examples::*;
// Not a separate `futures::StreamExt` import: `next` comes from the same
// prelude `.stream(..)` does.
use qbrs_sqlx::prelude::*;

const EXPORTED_TOTALS_FROM: i64 = 10_000;
const BATCH_SIZE: usize = 64;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    insert(orders::Table)
        .values_all((0..200i64).map(|n| {
            OrdersInsert::builder()
                .user_id(1)
                .total(EXPORTED_TOTALS_FROM + n)
                .build()
        }))
        .expect("two hundred orders")
        .execute(&pool)
        .await
        .expect("seed the orders");

    let export = select((orders::id, orders::total))
        .from(orders::Table)
        .filter(orders::total.gte(EXPORTED_TOTALS_FROM))
        .order_by(orders::id.asc());

    let mut rows = export.stream(&pool).expect("open the stream");
    let mut in_hand = Vec::new();
    let (mut written, mut flushes) = (0usize, 0usize);
    while let Some(row) = rows.next().await {
        let row = row.expect("decode a streamed row");
        in_hand.push(*row.get(orders::total));
        written += 1;
        if in_hand.len() == BATCH_SIZE {
            flushes += 1;
            in_hand.clear();
        }
    }
    if !in_hand.is_empty() {
        flushes += 1;
    }
    // Dropped before the next query rather than at the end of `main`: the
    // stream holds its connection until it is dropped.
    drop(rows);

    println!("wrote {written} rows in {flushes} flushes, holding at most {BATCH_SIZE}");
    assert_eq!(written, 200);

    let collected = export.load(&pool).await.expect("collect the same query");
    assert_eq!(
        collected.len(),
        written,
        "collecting the query yields the rows the stream did"
    );
}
