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
        .from::<Postgres, _>(orders::Table)
        .filter(orders::total.gt(1000i64));

    let q = select((big_orders::id, big_orders::total))
        .from_cte(qbrs::cte::with(big_orders::Table, &inner))
        .filter(big_orders::id.gt(0));

    let (sql, params) = q.to_sql();
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
        .from::<Postgres, _>(orders::Table)
        .filter(orders::total.gt(1000i64));
    let small = select((orders::id, orders::total))
        .from::<Postgres, _>(orders::Table)
        .filter(orders::total.lte(1000i64));

    let (sql, _params) = select((big_orders::id, small_orders::id))
        .from_cte(qbrs::cte::with(big_orders::Table, &big))
        .inner_join_cte(
            qbrs::cte::with(small_orders::Table, &small),
            small_orders::id.eq(big_orders::id),
        )
        .to_sql();
    assert_eq!(
        sql,
        "WITH \"big_orders\" (\"id\", \"total\") AS (SELECT \"orders\".\"id\", \"orders\".\"total\" FROM \"orders\" WHERE (\"orders\".\"total\" > $1)), \
         \"small_orders\" (\"id\", \"total\") AS (SELECT \"orders\".\"id\", \"orders\".\"total\" FROM \"orders\" WHERE (\"orders\".\"total\" <= $2)) \
         SELECT \"big_orders\".\"id\", \"small_orders\".\"id\" FROM \"big_orders\" INNER JOIN \"small_orders\" ON (\"small_orders\".\"id\" = \"big_orders\".\"id\")"
    );
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
