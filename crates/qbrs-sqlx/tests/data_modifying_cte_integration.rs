//! `WITH x AS (UPDATE .. RETURNING ..) SELECT ..` against a real Postgres.
//!
//! The shape exists to answer "write this row, and give me back a value the
//! row itself doesn't hold" in one round-trip. Splitting it into a write and
//! a read inside one transaction is correct but costs a round-trip on every
//! write, and pins a function to a concrete `Transaction` so the same
//! executor can be used twice.
//!
//! Two of its rules are Postgres's to confirm rather than the type system's:
//! every part of the statement sees one snapshot, and a data-modifying
//! `WITH` is legal only at the top level.

mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "realms")]
#[allow(dead_code)]
struct Realms {
    #[column(primary_key, generated)]
    id: i64,
    name: String,
}

#[derive(Table)]
#[table(name = "campaigns")]
#[allow(dead_code)]
struct Campaigns {
    #[column(primary_key, generated)]
    id: i64,
    name: String,
    realm_id: Option<i64>,
}

with! {
    struct updated { id: BigInt, name: Text, realm_id: Nullable<BigInt> }
}

/// What the outer query hands back: the written row, plus the joined name
/// it doesn't hold.
#[derive(qbrs::FromRow, Debug, PartialEq)]
struct Renamed {
    #[from_row(from = updated::name)]
    name: String,
    #[from_row(from = realms::name)]
    realm_name: String,
}

#[tokio::test]
async fn a_write_runs_as_a_cte_body_and_the_query_reads_what_the_row_doesnt_hold() {
    let (pool, guard) = common::test_pool("qbrs_data_modifying_cte").await;

    sqlx::query("DROP TABLE IF EXISTS campaigns")
        .execute(&pool)
        .await
        .expect("drop campaigns");
    sqlx::query("DROP TABLE IF EXISTS realms")
        .execute(&pool)
        .await
        .expect("drop realms");
    sqlx::query("CREATE TABLE realms (id BIGSERIAL PRIMARY KEY, name TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("create realms");
    sqlx::query(
        "CREATE TABLE campaigns (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL,
            realm_id BIGINT REFERENCES realms(id)
        )",
    )
    .execute(&pool)
    .await
    .expect("create campaigns");

    let realm_id: i64 = qbrs::insert::insert(realms::Table)
        .values(RealmsInsert::builder().name("west").build())
        .returning(realms::id)
        .load_one(&pool)
        .await
        .expect("insert a realm")
        .expect("returning row");
    let campaign_id: i64 = qbrs::insert::insert(campaigns::Table)
        .values(
            CampaignsInsert::builder()
                .name("spring")
                .realm_id(realm_id)
                .build(),
        )
        .returning(campaigns::id)
        .load_one(&pool)
        .await
        .expect("insert a campaign")
        .expect("returning row");

    let rename = qbrs::update::update(campaigns::Table)
        .set_to(campaigns::name, "summer")
        .filter(campaigns::id.eq(campaign_id))
        .returning((campaigns::id, campaigns::name, campaigns::realm_id));

    let renamed: Renamed = select((updated::name, realms::name))
        .from(qbrs::cte::with(updated::Table, &rename))
        .inner_join(realms::Table, realms::id.eq(updated::realm_id))
        .load_one(&pool)
        .await
        .expect("rename and read the realm's name in one statement")
        .expect("one row")
        .into_struct();
    assert_eq!(
        renamed,
        Renamed {
            name: "summer".to_string(),
            realm_name: "west".to_string(),
        }
    );

    // The write really happened, and not only in the row the CTE returned.
    let stored: String = select(campaigns::name)
        .from(campaigns::Table)
        .filter(campaigns::id.eq(campaign_id))
        .load_one(&pool)
        .await
        .expect("read the campaign back")
        .expect("one row");
    assert_eq!(stored, "summer");

    // One snapshot for the whole statement: the outer query joining the
    // *written* table sees the row as it was before the body touched it,
    // which is Postgres's rule and the reason the CTE returns its own rows.
    let before: String = select(campaigns::name)
        .from(qbrs::cte::with(
            updated::Table,
            &qbrs::update::update(campaigns::Table)
                .set_to(campaigns::name, "autumn")
                .filter(campaigns::id.eq(campaign_id))
                .returning((campaigns::id, campaigns::name, campaigns::realm_id)),
        ))
        .inner_join(campaigns::Table, campaigns::id.eq(updated::id))
        .load_one(&pool)
        .await
        .expect("read the written table from the same statement")
        .expect("one row");
    assert_eq!(before, "summer");

    // A row whose foreign key is NULL is the case the two-statement
    // workaround handles by skipping the second query. Here it is a
    // `LEFT JOIN` off the CTE, so one statement covers both rows — which
    // is what a `RETURNING` naming a joined column would have been for.
    let unattached: i64 = qbrs::insert::insert(campaigns::Table)
        .values(CampaignsInsert::builder().name("orphan").build())
        .returning(campaigns::id)
        .load_one(&pool)
        .await
        .expect("insert a campaign with no realm")
        .expect("returning row");

    let both: Vec<(String, Option<String>)> = select((updated::name, realms::name))
        .from(qbrs::cte::with(
            updated::Table,
            &qbrs::update::update(campaigns::Table)
                .set_to(campaigns::name, "renamed")
                .returning((campaigns::id, campaigns::name, campaigns::realm_id)),
        ))
        .left_join(realms::Table, realms::id.eq(updated::realm_id))
        .order_by(updated::id.asc())
        .load(&pool)
        .await
        .expect("rename both and read whatever realm each has")
        .into_tuples();
    assert_eq!(
        both,
        vec![
            ("renamed".to_string(), Some("west".to_string())),
            ("renamed".to_string(), None),
        ]
    );
    let _ = unattached;

    // Postgres takes a data-modifying `WITH` at the top level only, so the
    // query binding one has to be the statement. Nothing in the types says
    // so — this pins which error a nested one is, and that it is the
    // server's rather than a wrong answer.
    let nested = qbrs::update::update(campaigns::Table)
        .set_to(campaigns::name, "winter")
        .returning((campaigns::id, campaigns::name, campaigns::realm_id));
    let inner = select((updated::id,)).from(qbrs::cte::with(updated::Table, &nested));
    let refused = select(realms::id)
        .from(realms::Table)
        .filter(inner.exists())
        .load(&pool)
        .await
        .expect_err("a write CTE below the top level");
    let qbrs_sqlx::Error::Sqlx(sqlx::Error::Database(db)) = refused else {
        panic!("expected a database error, got {refused:?}");
    };
    assert_eq!(db.code().as_deref(), Some("0A000"));

    // And it really was refused: the row still holds what the last write
    // that did run left it.
    let untouched: String = select(campaigns::name)
        .from(campaigns::Table)
        .filter(campaigns::id.eq(campaign_id))
        .load_one(&pool)
        .await
        .expect("read the campaign back")
        .expect("one row");
    assert_eq!(untouched, "renamed");

    common::shutdown(pool, guard).await;
}
