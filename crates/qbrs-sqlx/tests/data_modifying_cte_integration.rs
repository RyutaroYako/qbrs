mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;
use sqlx::PgPool;

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

#[derive(qbrs::FromRow, Debug, PartialEq)]
struct RenamedWithRealm {
    #[from_row(from = updated::name)]
    name: String,
    #[from_row(from = realms::name)]
    realm_name: String,
}

#[tokio::test]
async fn an_update_runs_as_a_cte_body_whose_rows_the_outer_query_joins_in_one_statement() {
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

    a_cte_update_returns_its_row_joined_to_a_name_the_row_does_not_hold(&pool, campaign_id).await;
    assert_eq!(stored_campaign_name(&pool, campaign_id).await, "summer");

    the_outer_query_sees_the_written_table_as_it_was_before_the_cte_body_wrote_it(
        &pool,
        campaign_id,
    )
    .await;
    assert_eq!(stored_campaign_name(&pool, campaign_id).await, "autumn");

    a_left_join_on_a_cte_body_keeps_a_written_row_whose_foreign_key_is_null(&pool).await;

    postgres_refuses_a_data_modifying_cte_below_the_top_level_though_the_types_allow_it(&pool)
        .await;
    assert_eq!(stored_campaign_name(&pool, campaign_id).await, "renamed");

    common::shutdown(pool, guard).await;
}

fn campaign_update(new_name: &'static str) -> Update<Postgres, campaigns::Table> {
    qbrs::update::update(campaigns::Table).set_to(campaigns::name, new_name)
}

async fn stored_campaign_name(pool: &PgPool, campaign_id: i64) -> String {
    select(campaigns::name)
        .from(campaigns::Table)
        .filter(campaigns::id.eq(campaign_id))
        .load_one(pool)
        .await
        .expect("read the campaign back")
        .expect("one row")
}

async fn a_cte_update_returns_its_row_joined_to_a_name_the_row_does_not_hold(
    pool: &PgPool,
    campaign_id: i64,
) {
    let rename = campaign_update("summer")
        .filter(campaigns::id.eq(campaign_id))
        .returning((campaigns::id, campaigns::name, campaigns::realm_id));

    let renamed: RenamedWithRealm = select((updated::name, realms::name))
        .from(qbrs::cte::with(updated::Table, &rename))
        .inner_join(realms::Table, realms::id.eq(updated::realm_id))
        .load_one(pool)
        .await
        .expect("rename and read the realm's name in one statement")
        .expect("one row")
        .into_struct();
    assert_eq!(
        renamed,
        RenamedWithRealm {
            name: "summer".to_string(),
            realm_name: "west".to_string(),
        }
    );
}

async fn the_outer_query_sees_the_written_table_as_it_was_before_the_cte_body_wrote_it(
    pool: &PgPool,
    campaign_id: i64,
) {
    let before: String = select(campaigns::name)
        .from(qbrs::cte::with(
            updated::Table,
            &campaign_update("autumn")
                .filter(campaigns::id.eq(campaign_id))
                .returning((campaigns::id, campaigns::name, campaigns::realm_id)),
        ))
        .inner_join(campaigns::Table, campaigns::id.eq(updated::id))
        .load_one(pool)
        .await
        .expect("read the written table from the same statement")
        .expect("one row");
    assert_eq!(before, "summer");
}

async fn a_left_join_on_a_cte_body_keeps_a_written_row_whose_foreign_key_is_null(pool: &PgPool) {
    qbrs::insert::insert(campaigns::Table)
        .values(CampaignsInsert::builder().name("orphan").build())
        .execute(pool)
        .await
        .expect("insert a campaign with no realm");

    let both: Vec<(String, Option<String>)> = select((updated::name, realms::name))
        .from(qbrs::cte::with(
            updated::Table,
            &campaign_update("renamed").returning((
                campaigns::id,
                campaigns::name,
                campaigns::realm_id,
            )),
        ))
        .left_join(realms::Table, realms::id.eq(updated::realm_id))
        .order_by(updated::id.asc())
        .load(pool)
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
}

async fn postgres_refuses_a_data_modifying_cte_below_the_top_level_though_the_types_allow_it(
    pool: &PgPool,
) {
    let nested =
        campaign_update("winter").returning((campaigns::id, campaigns::name, campaigns::realm_id));
    let inner = select((updated::id,)).from(qbrs::cte::with(updated::Table, &nested));
    let refused = select(realms::id)
        .from(realms::Table)
        .filter(inner.exists())
        .load(pool)
        .await
        .expect_err("a write CTE below the top level");
    let qbrs_sqlx::Error::Sqlx(sqlx::Error::Database(db)) = refused else {
        panic!("expected a database error, got {refused:?}");
    };
    let feature_not_supported = Some("0A000");
    assert_eq!(db.code().as_deref(), feature_not_supported);
}
