mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "tickets")]
#[allow(dead_code)]
struct Tickets {
    #[column(primary_key, generated)]
    id: i64,
    subject: String,
    closed: bool,
}

#[derive(Table)]
#[table(name = "archived_tickets")]
#[allow(dead_code)]
struct ArchivedTickets {
    #[column(primary_key, generated)]
    id: i64,
    subject: String,
    closed: bool,
}

#[tokio::test]
async fn insert_select_copies_only_matching_rows_and_returns_the_generated_keys_postgres_filled() {
    let (pool, guard) = common::test_pool("qbrs_insert_select").await;

    for ddl in [
        "DROP TABLE IF EXISTS archived_tickets",
        "DROP TABLE IF EXISTS tickets",
        "CREATE TABLE tickets (id BIGSERIAL PRIMARY KEY, subject TEXT NOT NULL, closed BOOLEAN NOT NULL)",
        "CREATE TABLE archived_tickets (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            subject TEXT NOT NULL,
            closed BOOLEAN NOT NULL
        )",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(ddl))
            .execute(&pool)
            .await
            .expect("prepare the schema");
    }

    qbrs::insert::insert(tickets::Table)
        .values_all(
            [
                ("stuck door", true),
                ("flat tyre", false),
                ("dead battery", true),
            ]
            .map(|(subject, closed)| {
                TicketsInsert::builder()
                    .subject(subject)
                    .closed(closed)
                    .build()
            }),
        )
        .expect("three tickets")
        .execute(&pool)
        .await
        .expect("seed tickets");

    let closed_ones = select((tickets::subject, tickets::closed))
        .from(tickets::Table)
        .filter(tickets::closed.eq(true));
    let copied = qbrs::insert::insert(archived_tickets::Table)
        .select(&closed_ones)
        .execute(&pool)
        .await
        .expect("copy the closed tickets into the archive");
    assert_eq!(copied, 2);

    let archived: Vec<(i64, String)> = select((archived_tickets::id, archived_tickets::subject))
        .from(archived_tickets::Table)
        .order_by(archived_tickets::id.asc())
        .load(&pool)
        .await
        .expect("read the archive")
        .into_tuples();
    assert_eq!(
        archived.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
        vec!["stuck door", "dead battery"]
    );

    let second_pass: Vec<i64> = qbrs::insert::insert(archived_tickets::Table)
        .select(
            &select((tickets::subject, tickets::closed))
                .from(tickets::Table)
                .filter(tickets::closed.eq(false)),
        )
        .returning(archived_tickets::id)
        .load(&pool)
        .await
        .expect("copy the rest, returning their ids");
    assert_eq!(second_pass.len(), 1);

    common::shutdown(pool, guard).await;
}
