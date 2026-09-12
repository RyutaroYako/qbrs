//! A `sql!{}` expression carrying a bind, selected *and* grouped by *and*
//! ordered by in one statement. Postgres decides this one: its `GROUP BY`
//! check matches expressions syntactically, so two occurrences of the same
//! expression carrying differently-numbered placeholders are two different
//! expressions to it, and the statement is rejected with 42803. Only a real
//! server says whether the rendered SQL is one expression or three.

mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "readings")]
#[allow(dead_code)]
struct Readings {
    #[column(primary_key, generated)]
    id: i64,
    value: i64,
}

#[tokio::test]
async fn an_expression_carrying_a_bind_can_be_selected_and_grouped_by() {
    let (pool, guard) = common::test_pool("qbrs_reused_bind").await;

    sqlx::query("DROP TABLE IF EXISTS readings")
        .execute(&pool)
        .await
        .expect("drop readings");
    sqlx::query("CREATE TABLE readings (id BIGSERIAL PRIMARY KEY, value BIGINT NOT NULL)")
        .execute(&pool)
        .await
        .expect("create readings");

    qbrs::insert::insert(readings::Table)
        .values_all([3i64, 7, 12, 25].map(|value| ReadingsInsert::builder().value(value).build()))
        .expect("four readings")
        .execute(&pool)
        .await
        .expect("insert readings");

    let bucket = qbrs::sql!(
        qbrs::expr::BigInt,
        "((? / ?) * ?)",
        readings::value,
        10i64,
        10i64
    );
    let buckets: Vec<(i64, i64)> = select((bucket.clone(), qbrs::expr::count()))
        .from(readings::Table)
        .group_by(bucket.clone())
        .order_by(bucket.asc())
        .load(&pool)
        .await
        .expect("group by the same expression that is selected")
        .into_tuples();
    assert_eq!(buckets, vec![(0, 2), (10, 1), (20, 1)]);

    common::shutdown(pool, guard).await;
}
