//! The column types a database has and Rust doesn't, executed end to end:
//! rendered, bound, and decoded back into the type the schema declared.
//! Runs under `--all-features`; each type is behind the feature that names
//! the crate it decodes to.
#![cfg(all(feature = "chrono", feature = "uuid", feature = "decimal"))]

mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "events")]
#[allow(dead_code)]
struct Events {
    #[column(primary_key)]
    id: uuid::Uuid,
    happened_at: chrono::DateTime<chrono::Utc>,
    on_day: Option<chrono::NaiveDate>,
    amount: rust_decimal::Decimal,
}

#[tokio::test]
async fn timestamp_uuid_and_numeric_columns_survive_a_round_trip() {
    let (pool, guard) = common::test_pool("qbrs_column_types").await;

    sqlx::query("DROP TABLE IF EXISTS events")
        .execute(&pool)
        .await
        .expect("drop events");
    sqlx::query(
        "CREATE TABLE events (
            id UUID PRIMARY KEY,
            happened_at TIMESTAMPTZ NOT NULL,
            on_day DATE,
            amount NUMERIC NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("create events");

    let id = uuid::Uuid::from_u128(0x0192_3f7c_dead_beef_0000_0000_0000_0001);
    let happened_at = chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("a timestamp");
    let on_day = chrono::NaiveDate::from_ymd_opt(2024, 5, 17).expect("a date");
    let amount = "12.34".parse::<rust_decimal::Decimal>().expect("a decimal");

    let inserted = insert(events::Table)
        .values(
            EventsInsert::builder()
                .id(id)
                .happened_at(happened_at)
                .amount(amount)
                .on_day(on_day)
                .build(),
        )
        .returning(events::All)
        .load_one(&pool)
        .await
        .expect("insert an event")
        .expect("RETURNING gave a row");

    assert_eq!(inserted.get(events::id), &id);
    assert_eq!(inserted.get(events::happened_at), &happened_at);
    assert_eq!(inserted.get(events::on_day), &Some(on_day));
    assert_eq!(inserted.get(events::amount), &amount);

    // And they are ordinary values to the builder: comparable, bindable.
    let found: Vec<uuid::Uuid> = select(events::id)
        .from(events::Table)
        .filter(events::happened_at.lte(happened_at))
        .filter(events::amount.gt("1.00".parse::<rust_decimal::Decimal>().unwrap()))
        .load(&pool)
        .await
        .expect("select by timestamp and amount");
    assert_eq!(found, vec![id]);

    // A money column totals as money and averages as a float, which is what
    // the row declares in each case.
    let (total, mean): (Option<rust_decimal::Decimal>, Option<f64>) =
        select((sum(events::amount), avg(events::amount)))
            .from(events::Table)
            .load_one(&pool)
            .await
            .expect("aggregate a numeric column")
            .expect("one row")
            .into_tuple();
    assert_eq!(total, Some(amount));
    assert_eq!(mean, Some(12.34));

    common::shutdown(pool, guard).await;
}
