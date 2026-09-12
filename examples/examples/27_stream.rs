//! `.stream(..)`: the rows one at a time, for a result too large to hold —
//! an export that writes as it reads rather than collecting first. Not a
//! cursor: the server still produces the whole result and the connection is
//! held until the stream ends; what this bounds is the client's memory.
//! Run: `cargo run -p qbrs-examples --example 27_stream`

use qbrs::prelude::*;
use qbrs_examples::*;
// `StreamExt::next` comes from the same prelude `.stream(..)` does.
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    // Enough rows that collecting them would be the wrong shape for an
    // export, even though this one is small enough to check afterwards.
    // Their totals start well above the seed's, so the filter below picks
    // out exactly these.
    insert(orders::Table)
        .values_all(
            (0..200i64).map(|n| OrdersInsert::builder().user_id(1).total(10_000 + n).build()),
        )
        .expect("two hundred orders")
        .execute(&pool)
        .await
        .expect("seed the orders");

    let export = select((orders::id, orders::total))
        .from(orders::Table)
        .filter(orders::total.gte(10_000i64))
        .order_by(orders::id.asc());

    // The rows arrive as they are read. A real export would write each
    // batch out and drop it; this one counts what it wrote.
    let mut rows = export.stream(&pool).expect("open the stream");
    let mut in_hand = Vec::new();
    let (mut written, mut flushes) = (0usize, 0usize);
    while let Some(row) = rows.next().await {
        let row = row.expect("decode a streamed row");
        in_hand.push(*row.get(orders::total));
        written += 1;
        if in_hand.len() == 64 {
            // ... write `in_hand` to the file or the upload ...
            flushes += 1;
            in_hand.clear();
        }
    }
    if !in_hand.is_empty() {
        flushes += 1;
    }
    // The stream borrows the connection until it is done with it.
    drop(rows);

    println!("wrote {written} rows in {flushes} flushes, holding at most 64");
    assert_eq!(written, 200);

    // The same query collected: streaming and collecting differ in when
    // the rows arrive, not in what they are.
    let collected = export.load(&pool).await.expect("collect the same query");
    assert_eq!(collected.len(), written);
}
