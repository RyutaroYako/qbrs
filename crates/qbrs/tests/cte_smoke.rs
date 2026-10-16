use qbrs::prelude::*;
use qbrs::select::Selection;
use std::marker::PhantomData;

#[derive(Table)]
#[table(name = "orders")]
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i32,
    total: i64,
}

with! {
    struct big_orders { id: Integer, total: BigInt }
}

with! {
    struct small_orders { id: Integer, total: BigInt }
}

#[test]
fn with_binds_a_named_cte_that_is_selected_and_filtered_like_a_real_table() {
    let inner = select((orders::id, orders::total))
        .from(orders::Table)
        .filter(orders::total.gt(1000i64));

    let q = select((big_orders::id, big_orders::total))
        .from(qbrs::cte::with(big_orders::Table, &inner))
        .filter(big_orders::id.gt(0));

    let (sql, params) = q.to_sql(Postgres);
    assert_eq!(
        sql,
        "WITH \"big_orders\" (\"id\", \"total\") AS (SELECT \"orders\".\"id\", \"orders\".\"total\" FROM \"orders\" WHERE (\"orders\".\"total\" > $1)) \
         SELECT \"big_orders\".\"id\", \"big_orders\".\"total\" FROM \"big_orders\" WHERE (\"big_orders\".\"id\" > $2)"
    );
    assert_eq!(
        params,
        vec![qbrs::expr::Value::I64(1000), qbrs::expr::Value::I32(0)]
    );
}

#[test]
fn two_independent_ctes_share_one_with_clause_separated_by_a_comma() {
    let big = select((orders::id, orders::total))
        .from(orders::Table)
        .filter(orders::total.gt(1000i64));
    let small = select((orders::id, orders::total))
        .from(orders::Table)
        .filter(orders::total.lte(1000i64));

    let (sql, _params) = select((big_orders::id, small_orders::id))
        .from(qbrs::cte::with(big_orders::Table, &big))
        .inner_join(
            qbrs::cte::with(small_orders::Table, &small),
            small_orders::id.eq(big_orders::id),
        )
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "WITH \"big_orders\" (\"id\", \"total\") AS (SELECT \"orders\".\"id\", \"orders\".\"total\" FROM \"orders\" WHERE (\"orders\".\"total\" > $1)), \
         \"small_orders\" (\"id\", \"total\") AS (SELECT \"orders\".\"id\", \"orders\".\"total\" FROM \"orders\" WHERE (\"orders\".\"total\" <= $1)) \
         SELECT \"big_orders\".\"id\", \"small_orders\".\"id\" FROM \"big_orders\" INNER JOIN \"small_orders\" ON (\"small_orders\".\"id\" = \"big_orders\".\"id\")"
    );
}

#[test]
fn two_cte_bodies_binding_the_same_value_share_one_parameter() {
    let big = select((orders::id, orders::total))
        .from(orders::Table)
        .filter(orders::total.gt(1000i64));
    let small = select((orders::id, orders::total))
        .from(orders::Table)
        .filter(orders::total.lte(1000i64));

    let (sql, params) = select((big_orders::id, small_orders::id))
        .from(qbrs::cte::with(big_orders::Table, &big))
        .inner_join(
            qbrs::cte::with(small_orders::Table, &small),
            small_orders::id.eq(big_orders::id),
        )
        .to_sql(Postgres);
    assert_eq!(sql.matches("$1").count(), 2);
    assert!(!sql.contains("$2"), "{sql}");
    assert_eq!(params, vec![qbrs::expr::Value::I64(1000)]);
}

#[test]
fn a_cte_column_is_read_by_key_and_by_accessor_like_a_table_column() {
    use qbrs::row::{Row, RowCons, RowNil};
    let row = Row::new(RowCons::<big_orders::columns::total, _, _>::new(
        2500i64, RowNil,
    ));
    assert_eq!(row.get(big_orders::total), &2500);
    assert_eq!(row.total(), &2500);
}

with! {
    struct managers { id: Integer, name: Text }
}

#[derive(Table)]
#[table(name = "employees")]
#[allow(dead_code)]
struct Employees {
    #[column(primary_key, generated)]
    id: i32,
    name: String,
    manager_id: Option<i32>,
}

#[test]
fn a_cte_over_the_same_table_stands_in_for_an_aliased_self_join() {
    let manager_rows = select((employees::id, employees::name)).from(employees::Table);

    let (sql, _params) = select((employees::name, managers::name))
        .from(employees::Table)
        .inner_join(
            qbrs::cte::with(managers::Table, &manager_rows),
            managers::id.eq(employees::manager_id),
        )
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "WITH \"managers\" (\"id\", \"name\") AS (SELECT \"employees\".\"id\", \"employees\".\"name\" FROM \"employees\") \
         SELECT \"employees\".\"name\", \"managers\".\"name\" FROM \"employees\" \
         INNER JOIN \"managers\" ON (\"managers\".\"id\" = \"employees\".\"manager_id\")"
    );
}

with! {
    struct raised { id: Integer, total: BigInt }
}

#[test]
fn an_update_returning_binds_as_a_cte_body_whose_rows_the_outer_select_joins() {
    let bump = qbrs::update::update(orders::Table)
        .set_to(orders::total, 2000i64)
        .filter(orders::id.eq(1))
        .returning((orders::id, orders::total));

    let (sql, params) = select((raised::id, raised::total, orders::total))
        .from(qbrs::cte::with(raised::Table, &bump))
        .inner_join(orders::Table, orders::id.eq(raised::id))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "WITH \"raised\" (\"id\", \"total\") AS (UPDATE \"orders\" SET \"total\" = $1 WHERE (\"orders\".\"id\" = $2) \
         RETURNING \"orders\".\"id\", \"orders\".\"total\") \
         SELECT \"raised\".\"id\", \"raised\".\"total\", \"orders\".\"total\" FROM \"raised\" \
         INNER JOIN \"orders\" ON (\"orders\".\"id\" = \"raised\".\"id\")"
    );
    assert_eq!(
        params,
        vec![qbrs::expr::Value::I64(2000), qbrs::expr::Value::I32(1)]
    );
}

#[test]
fn a_write_cte_body_numbers_its_binds_before_the_outer_querys() {
    let bump = qbrs::insert::insert(orders::Table)
        .values(OrdersInsert::builder().total(500i64).build())
        .returning((orders::id, orders::total));

    let (sql, params) = select((raised::id,))
        .from(qbrs::cte::with(raised::Table, &bump))
        .filter(raised::total.gt(100i64))
        .to_sql(Postgres);
    assert!(
        sql.starts_with(
            "WITH \"raised\" (\"id\", \"total\") AS (INSERT INTO \"orders\" (\"total\") VALUES ($1) \
             RETURNING \"orders\".\"id\", \"orders\".\"total\") "
        ),
        "{sql}"
    );
    assert!(sql.ends_with("WHERE (\"raised\".\"total\" > $2)"), "{sql}");
    assert_eq!(
        params,
        vec![qbrs::expr::Value::I64(500), qbrs::expr::Value::I64(100)]
    );
}

#[test]
fn all_row_is_the_row_type_select_all_decodes_to_from_a_cte() {
    fn output_of<D, Scope, Sel, Idx>(_: &Select<D, Scope, Sel>, _: D) -> PhantomData<Sel::Output>
    where
        Sel: Selection<Scope, Idx>,
    {
        PhantomData
    }

    let body = select((orders::id, orders::total)).from(orders::Table);
    let _: PhantomData<big_orders::AllRow> = output_of(
        &select(big_orders::All).from(qbrs::cte::with(big_orders::Table, &body)),
        Postgres,
    );
}

with! {
    struct ranked_orders { id: Integer, rank_by_total: BigInt }
}

#[test]
fn a_rank_is_filtered_through_a_cte_that_selects_it_under_its_column_name() {
    label!(rank_by_total);
    let ranked = select((
        orders::id,
        qbrs::window::row_number()
            .over(qbrs::window::window().order_by(orders::total.desc()))
            .label(label::rank_by_total),
    ))
    .from(orders::Table);

    let (sql, _params) = select(ranked_orders::id)
        .from(qbrs::cte::with(ranked_orders::Table, &ranked))
        .filter(ranked_orders::rank_by_total.lte(3i64))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "WITH \"ranked_orders\" (\"id\", \"rank_by_total\") AS (SELECT \"orders\".\"id\", row_number() OVER (ORDER BY \"orders\".\"total\" DESC) AS \"rank_by_total\" FROM \"orders\") \
         SELECT \"ranked_orders\".\"id\" FROM \"ranked_orders\" WHERE (\"ranked_orders\".\"rank_by_total\" <= $1)"
    );
}
