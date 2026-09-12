//! `with!{}` + `cte::with()`: `WITH name (cols..) AS (..) SELECT .. FROM name ..`.
//! Lives here rather than in `qbrs-core` because `with!` is a proc macro, for
//! the same reason `#[derive(Table)]` is: it synthesizes identifiers.

use qbrs::prelude::*;

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
fn with_binds_a_named_cte_usable_as_a_real_table() {
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
fn multiple_independent_ctes_render_comma_separated() {
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

/// A CTE body is rendered to a `Fragment` before the host query numbers
/// anything, so its binds reach the statement through `splice_into`. Two
/// bodies binding the same value therefore have to meet in the host's
/// parameter list, not in either fragment.
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
fn a_cte_column_is_read_by_key_and_by_accessor() {
    // `with!` now generates the same accessor traits `#[derive(Table)]` does,
    // so a CTE column is read exactly like a real one.
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
fn a_cte_of_the_same_table_stands_in_for_a_self_join() {
    // qbrs has no table aliasing (see README's Known limitations), so a
    // literal `FROM "employees" AS e JOIN "employees" AS m` can't be
    // written. Binding a `with!{}` pseudo-table to a plain `SELECT` over the
    // *same* table gets the same result: each employee row paired with its
    // own manager's row, fully type-checked.
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

/// The shape the write-then-read round-trip collapses into: the `UPDATE`
/// runs as the CTE body and the outer `SELECT` reads its rows, joined to
/// whatever the written row itself doesn't hold.
#[test]
fn a_write_statement_binds_as_a_cte_body_and_the_query_reads_its_rows() {
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

/// A CTE body's placeholders are numbered by the statement it lands in, so
/// a write body's binds count from where the host query has got to — the
/// same rule a `SELECT` body follows, and the reason a body is a `Fragment`.
#[test]
fn a_write_body_is_numbered_by_the_statement_it_lands_in() {
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
fn a_cte_names_its_row_the_way_a_table_does() {
    // `with!` generates `AllRow` for the same reason `#[derive(Table)]`
    // does: a stored query over `select(All)` names a row rather than
    // spelling a `RowCons` chain.
    fn total(row: big_orders::AllRow) -> i64 {
        *row.get(big_orders::total)
    }
    let _ = total;
}
