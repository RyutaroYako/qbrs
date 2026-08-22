//! `with!{}` + `cte::with()`: `WITH name (cols..) AS (..) SELECT .. FROM name ..`.

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::ExprMethods;
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::select;
use qbrs_core::with;

pub struct OrdersMarker;
impl TableTrait for OrdersMarker {
    const NAME: &'static str = "orders";
}

#[allow(non_upper_case_globals)]
mod orders {
    use super::OrdersMarker;
    use qbrs_core::expr::{BigInt, Column, Integer};

    pub const Table: OrdersMarker = OrdersMarker;
    #[allow(non_camel_case_types)]
    pub mod columns {
        use super::*;
        use qbrs_core::expr::ColumnKey;
        #[derive(Clone, Copy)]
        pub struct id;
        impl ColumnKey for id {
            type Table = OrdersMarker;
            type Sql = Integer;
            const NAME: &'static str = "id";
        }
        #[derive(Clone, Copy)]
        pub struct total;
        impl ColumnKey for total {
            type Table = OrdersMarker;
            type Sql = BigInt;
            const NAME: &'static str = "total";
        }
    }

    pub const id: Column<columns::id> = Column::new();
    pub const total: Column<columns::total> = Column::new();
}

with! {
    struct big_orders { id: qbrs_core::expr::Integer, total: qbrs_core::expr::BigInt }
}

#[test]
fn with_binds_a_named_cte_usable_as_a_real_table() {
    let inner = select((orders::id, orders::total))
        .from::<Postgres, _>(orders::Table)
        .filter(orders::total.gt(1000i64));

    let q = select((big_orders::id, big_orders::total))
        .with(qbrs_core::cte::with(big_orders::Table, &inner))
        .from::<Postgres, _>(big_orders::Table)
        .filter(big_orders::id.gt(0));

    let (sql, params) = q.to_sql();
    assert_eq!(
        sql,
        "WITH \"big_orders\" (\"id\", \"total\") AS (SELECT \"orders\".\"id\", \"orders\".\"total\" FROM \"orders\" WHERE (\"orders\".\"total\" > $1)) \
         SELECT \"big_orders\".\"id\", \"big_orders\".\"total\" FROM \"big_orders\" WHERE (\"big_orders\".\"id\" > $2)"
    );
    assert_eq!(
        params,
        vec![
            qbrs_core::expr::Value::I64(1000),
            qbrs_core::expr::Value::I32(0)
        ]
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

    let (sql, _params) = select((big_orders::id,))
        .with(qbrs_core::cte::with(big_orders::Table, &big))
        .with(qbrs_core::cte::with(small_orders::Table, &small))
        .from::<Postgres, _>(big_orders::Table)
        .to_sql();
    assert_eq!(
        sql,
        "WITH \"big_orders\" (\"id\", \"total\") AS (SELECT \"orders\".\"id\", \"orders\".\"total\" FROM \"orders\" WHERE (\"orders\".\"total\" > $1)), \
         \"small_orders\" (\"id\", \"total\") AS (SELECT \"orders\".\"id\", \"orders\".\"total\" FROM \"orders\" WHERE (\"orders\".\"total\" <= $2)) \
         SELECT \"big_orders\".\"id\" FROM \"big_orders\""
    );
}

with! {
    struct small_orders { id: qbrs_core::expr::Integer, total: qbrs_core::expr::BigInt }
}

// Uncomment to eyeball the compile error for a CTE body whose actual SELECT
// list doesn't match its `with!{}`-declared shape (confirmed working — kept
// out of the normal test run since it's meant to fail): `big_orders`
// declares `(i32, i64)` but this query only selects one `BigInt` column.
//
// #[test]
// fn mismatched_cte_shape_is_a_compile_error() {
//     let inner = select((orders::total,)).from::<Postgres, _>(orders::Table);
//     let _ = qbrs_core::cte::with(big_orders::Table, &inner);
//     // error[E0271]: type mismatch resolving `<(Column<...>,) as Selection<...>>::Output == (i32, i64)`
// }
