mod common;

use qbrs::Table;
use qbrs::expr::ExprMethods;
use qbrs::row::{IntoStructs, IntoTuples};
use qbrs::select::{OrderExt, select};
use qbrs::statement::Statement;
use qbrs::update::Assignments;
use qbrs_sqlx::{CountExt, ExecuteExt, LoadExt};
use sqlx::PgPool;

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
    #[column(default)]
    active: bool,
}

#[derive(qbrs::FromRow, Debug)]
#[allow(dead_code)]
struct UserRow {
    id: i64,
    email: String,
    display_name: Option<String>,
    active: bool,
}

#[derive(Table)]
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

#[tokio::test]
async fn insert_select_update_delete_and_count_round_trip_through_postgres() {
    let (pool, guard) = common::test_pool("qbrs_test").await;

    sqlx::query("DROP TABLE IF EXISTS orders")
        .execute(&pool)
        .await
        .expect("drop orders table");
    sqlx::query("DROP TABLE IF EXISTS users")
        .execute(&pool)
        .await
        .expect("drop users table");

    sqlx::query(
        "CREATE TABLE users (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            email TEXT NOT NULL,
            display_name TEXT,
            active BOOLEAN NOT NULL DEFAULT true
        )",
    )
    .execute(&pool)
    .await
    .expect("create users table");

    sqlx::query(
        "CREATE TABLE orders (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            user_id BIGINT NOT NULL REFERENCES users(id),
            total BIGINT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("create orders table");

    let (ada_id, dan_id) = an_insert_returning_hands_back_one_generated_id_per_row(&pool).await;

    qbrs::insert::insert(orders::Table)
        .values(OrdersInsert::builder().user_id(ada_id).total(1000).build())
        .values(OrdersInsert::builder().user_id(ada_id).total(2500).build())
        .execute(&pool)
        .await
        .expect("insert orders");

    a_select_with_where_order_by_and_limit_returns_the_matching_rows_in_order(&pool).await;
    a_left_join_decodes_a_missing_right_row_as_none(&pool).await;
    an_update_from_a_row_changes_only_the_touched_column(&pool, ada_id).await;
    select_all_fills_a_from_row_struct_by_column_name(&pool, ada_id).await;
    a_left_joined_select_all_decodes_every_column_of_a_missing_row_as_none(&pool, dan_id).await;
    postgres_accepts_order_by_selected_on_a_distinct_select(&pool).await;
    order_by_selected_counts_a_column_behind_an_all_element_where_postgres_does(&pool).await;
    a_count_ignores_the_order_by_and_limit_of_the_query_it_counts(&pool).await;
    a_delete_removes_only_the_filtered_row(&pool, dan_id, ada_id).await;

    common::shutdown(pool, guard).await;
}

async fn an_insert_returning_hands_back_one_generated_id_per_row(pool: &PgPool) -> (i64, i64) {
    let inserted_ids: Vec<i64> = qbrs::insert::insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("ada@example.com")
                .display_name("Ada")
                .build(),
        )
        .values(UsersInsert::builder().email("dan@example.com").build())
        .returning(users::id)
        .load(pool)
        .await
        .expect("insert users");
    let [ada_id, dan_id] = inserted_ids[..] else {
        panic!("expected two ids, got {inserted_ids:?}");
    };
    (ada_id, dan_id)
}

async fn a_select_with_where_order_by_and_limit_returns_the_matching_rows_in_order(pool: &PgPool) {
    let names: Vec<String> = select(users::email)
        .from(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::id.asc())
        .limit(10)
        .load(pool)
        .await
        .expect("select users");
    assert_eq!(
        names,
        vec!["ada@example.com".to_string(), "dan@example.com".to_string()]
    );
}

async fn a_left_join_decodes_a_missing_right_row_as_none(pool: &PgPool) {
    let rows: Vec<(String, Option<i64>)> = select((users::email, orders::total))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(users::id.asc())
        .load(pool)
        .await
        .expect("left join select")
        .into_tuples();
    assert!(rows.contains(&("dan@example.com".to_string(), None)));
    assert!(rows.contains(&("ada@example.com".to_string(), Some(1000))));
    assert!(rows.contains(&("ada@example.com".to_string(), Some(2500))));
}

async fn an_update_from_a_row_changes_only_the_touched_column(pool: &PgPool, ada_id: i64) {
    let affected = qbrs::update::update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("Ada Lovelace".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
        )
        .filter(users::id.eq(ada_id))
        .execute(pool)
        .await
        .expect("update user");
    assert_eq!(affected, 1);

    let updated: (String, Option<String>) = select((users::email, users::display_name))
        .from(users::Table)
        .filter(users::id.eq(ada_id))
        .load_one(pool)
        .await
        .expect("select updated user")
        .expect("one row")
        .into_tuple();
    assert_eq!(
        updated,
        (
            "ada@example.com".to_string(),
            Some("Ada Lovelace".to_string())
        )
    );
}

async fn select_all_fills_a_from_row_struct_by_column_name(pool: &PgPool, ada_id: i64) {
    let whole: Vec<UserRow> = select(users::All)
        .from(users::Table)
        .filter(users::id.eq(ada_id))
        .load(pool)
        .await
        .expect("select every column")
        .into_structs();
    assert_eq!(whole.len(), 1);
    assert_eq!(whole[0].email, "ada@example.com");
    assert_eq!(whole[0].display_name.as_deref(), Some("Ada Lovelace"));
}

async fn a_left_joined_select_all_decodes_every_column_of_a_missing_row_as_none(
    pool: &PgPool,
    dan_id: i64,
) {
    let joined = select((users::email, orders::All))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::id.eq(dan_id))
        .load_one(pool)
        .await
        .expect("left join every order column")
        .expect("dan is there");
    assert_eq!(joined.get(users::email), "dan@example.com");
    assert_eq!(joined.get(orders::id), &None);
    assert_eq!(joined.get(orders::total), &None);
}

async fn postgres_accepts_order_by_selected_on_a_distinct_select(pool: &PgPool) {
    let buyers: Vec<(String, bool)> = select((users::email, users::active))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .distinct()
        .order_by_selected(users::email, qbrs::select::SortDir::Asc)
        .load(pool)
        .await
        .expect("distinct buyers sorted by a selected column")
        .into_tuples();
    assert_eq!(buyers, vec![("ada@example.com".to_string(), true)]);
}

async fn order_by_selected_counts_a_column_behind_an_all_element_where_postgres_does(
    pool: &PgPool,
) {
    let spends: Vec<i64> = select((users::All, orders::total))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .distinct()
        .order_by_selected(orders::total, qbrs::select::SortDir::Desc)
        .load(pool)
        .await
        .expect("distinct rows sorted by a column behind an All element")
        .into_iter()
        .map(|row| *row.get(orders::total))
        .collect();
    assert_eq!(spends, vec![2500, 1000]);
}

async fn a_delete_removes_only_the_filtered_row(pool: &PgPool, dan_id: i64, ada_id: i64) {
    let deleted = qbrs::delete::delete(users::Table)
        .filter(users::id.eq(dan_id))
        .execute(pool)
        .await
        .expect("delete user");
    assert_eq!(deleted, 1);

    let remaining: Vec<i64> = select(users::id)
        .from(users::Table)
        .load(pool)
        .await
        .expect("select remaining users");
    assert_eq!(remaining, vec![ada_id]);
}

async fn a_count_ignores_the_order_by_and_limit_of_the_query_it_counts(pool: &PgPool) {
    let total = select((users::id,))
        .from(users::Table)
        .order_by(users::id.desc())
        .limit(1)
        .count(pool)
        .await
        .expect("count users");
    assert_eq!(total, 2);
}
