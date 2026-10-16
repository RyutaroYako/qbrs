#![cfg(all(feature = "chrono", feature = "uuid", feature = "decimal"))]

mod common;

// Fully qualifying the field types below would stop this file from checking
// that the derive takes a type the caller named through its own `use`.
use chrono::{DateTime, Utc};
use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Table)]
#[table(name = "events")]
#[allow(dead_code)]
struct Events {
    #[column(primary_key)]
    id: Uuid,
    happened_at: DateTime<Utc>,
    on_day: Option<chrono::NaiveDate>,
    amount: Decimal,
}

#[derive(Debug, FromRow, PartialEq)]
struct EventDto {
    happened_at: DateTime<Utc>,
    amount: Decimal,
}

struct Event {
    id: Uuid,
    happened_at: DateTime<Utc>,
    on_day: chrono::NaiveDate,
    amount: Decimal,
}

#[tokio::test]
async fn timestamp_date_uuid_and_numeric_columns_bind_and_decode_as_the_schema_declares() {
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

    let event = Event {
        id: Uuid::from_u128(0x0192_3f7c_dead_beef_0000_0000_0000_0001),
        happened_at: DateTime::from_timestamp(1_700_000_000, 0).expect("a timestamp"),
        on_day: chrono::NaiveDate::from_ymd_opt(2024, 5, 17).expect("a date"),
        amount: "12.34".parse().expect("a decimal"),
    };

    insert_returning_all_decodes_each_column_as_its_declared_type(&pool, &event).await;
    timestamp_and_numeric_values_bind_as_comparison_operands(&pool, &event).await;
    timestamp_and_numeric_columns_fill_a_from_row_struct(&pool, &event).await;
    min_and_max_decode_a_timestamp_and_a_numeric_as_their_own_types(&pool, &event).await;
    sum_of_a_numeric_decodes_as_a_decimal_and_avg_as_a_float(&pool, &event).await;
    an_imported_uuid_is_the_marker_in_a_with_declaration_and_a_sql_fragment(&pool, &event).await;

    common::shutdown(pool, guard).await;
}

async fn insert_returning_all_decodes_each_column_as_its_declared_type(
    pool: &PgPool,
    event: &Event,
) {
    let inserted = insert(events::Table)
        .values(
            EventsInsert::builder()
                .id(event.id)
                .happened_at(event.happened_at)
                .amount(event.amount)
                .on_day(event.on_day)
                .build(),
        )
        .returning(events::All)
        .load_one(pool)
        .await
        .expect("insert an event")
        .expect("RETURNING gave a row");

    assert_eq!(inserted.get(events::id), &event.id);
    assert_eq!(inserted.get(events::happened_at), &event.happened_at);
    assert_eq!(inserted.get(events::on_day), &Some(event.on_day));
    assert_eq!(inserted.get(events::amount), &event.amount);
}

async fn timestamp_and_numeric_values_bind_as_comparison_operands(pool: &PgPool, event: &Event) {
    let found: Vec<Uuid> = select(events::id)
        .from(events::Table)
        .filter(events::happened_at.lte(event.happened_at))
        .filter(events::amount.gt("1.00".parse::<Decimal>().expect("a decimal")))
        .load(pool)
        .await
        .expect("select by timestamp and amount");
    assert_eq!(found, vec![event.id]);
}

async fn timestamp_and_numeric_columns_fill_a_from_row_struct(pool: &PgPool, event: &Event) {
    let dtos: Vec<EventDto> = select((events::happened_at, events::amount))
        .from(events::Table)
        .load(pool)
        .await
        .expect("select into a DTO")
        .into_structs();
    assert_eq!(
        dtos,
        vec![EventDto {
            happened_at: event.happened_at,
            amount: event.amount,
        }]
    );
}

async fn min_and_max_decode_a_timestamp_and_a_numeric_as_their_own_types(
    pool: &PgPool,
    event: &Event,
) {
    let (earliest, largest): (Option<DateTime<Utc>>, Option<Decimal>) =
        select((min(events::happened_at), max(events::amount)))
            .from(events::Table)
            .load_one(pool)
            .await
            .expect("min and max")
            .expect("one row")
            .into_tuple();
    assert_eq!(earliest, Some(event.happened_at));
    assert_eq!(largest, Some(event.amount));
}

async fn sum_of_a_numeric_decodes_as_a_decimal_and_avg_as_a_float(pool: &PgPool, event: &Event) {
    let (total, mean): (Option<Decimal>, Option<f64>) =
        select((sum(events::amount), avg(events::amount)))
            .from(events::Table)
            .load_one(pool)
            .await
            .expect("aggregate a numeric column")
            .expect("one row")
            .into_tuple();
    assert_eq!(total, Some(event.amount));
    assert_eq!(mean, Some(12.34));
}

async fn an_imported_uuid_is_the_marker_in_a_with_declaration_and_a_sql_fragment(
    pool: &PgPool,
    event: &Event,
) {
    with! {
        struct recent { id: Uuid }
    }
    let ids: Vec<Uuid> = select(recent::id)
        .from(qbrs::cte::with(
            recent::Table,
            &select((events::id,)).from(events::Table),
        ))
        .filter(recent::id.eq(qbrs::sql!(Uuid, "?", event.id)))
        .load(pool)
        .await
        .expect("a uuid in a CTE declaration and in a sql! fragment");
    assert_eq!(ids, vec![event.id]);
}
