//! `row_number()`/`rank()`/`dense_rank()` `.over(window()..)`.

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::ExprMethods;
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::{OrderExt, select};
use qbrs_core::window::{dense_rank, rank, row_number, window};

pub struct EmployeesMarker;
impl TableTrait for EmployeesMarker {
    const NAME: &'static str = "employees";
}
impl qbrs_core::scope::BaseTableSealed for EmployeesMarker {}
impl qbrs_core::scope::BaseTable for EmployeesMarker {}

pub struct OrdersMarker;
impl TableTrait for OrdersMarker {
    const NAME: &'static str = "orders";
}
impl qbrs_core::scope::BaseTableSealed for OrdersMarker {}
impl qbrs_core::scope::BaseTable for OrdersMarker {}

#[allow(non_upper_case_globals)]
mod employees {
    use super::EmployeesMarker;
    use qbrs_core::expr::{Column, Integer, Text};

    pub const Table: EmployeesMarker = EmployeesMarker;
    #[allow(non_camel_case_types)]
    pub mod columns {
        use super::*;
        use qbrs_core::expr::ColumnKey;
        #[derive(Clone, Copy)]
        pub struct department;
        impl qbrs_core::expr::WritableSealed for department {}
        impl qbrs_core::expr::Writable for department {}
        impl ColumnKey for department {
            type Table = EmployeesMarker;
            type Sql = Text;
        }
        impl qbrs_core::row::NamedSealed for department {}
        impl qbrs_core::row::Named for department {
            type Name = qbrs_core::type_name!('d', 'e', 'p', 'a', 'r', 't', 'm', 'e', 'n', 't');
            const NAME: &'static str = "department";
        }
        impl qbrs_core::row::Spelled for department {}
        #[derive(Clone, Copy)]
        pub struct salary;
        impl qbrs_core::expr::WritableSealed for salary {}
        impl qbrs_core::expr::Writable for salary {}
        impl ColumnKey for salary {
            type Table = EmployeesMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::NamedSealed for salary {}
        impl qbrs_core::row::Named for salary {
            type Name = qbrs_core::type_name!('s', 'a', 'l', 'a', 'r', 'y');
            const NAME: &'static str = "salary";
        }
        impl qbrs_core::row::Spelled for salary {}
    }

    pub const department: Column<columns::department> = Column::new();
    pub const salary: Column<columns::salary> = Column::new();
}

#[allow(non_upper_case_globals, dead_code)]
mod orders {
    use super::OrdersMarker;
    use qbrs_core::expr::{Column, Integer};

    #[allow(non_camel_case_types)]
    pub mod columns {
        use super::*;
        use qbrs_core::expr::ColumnKey;
        #[derive(Clone, Copy)]
        pub struct total;
        impl qbrs_core::expr::WritableSealed for total {}
        impl qbrs_core::expr::Writable for total {}
        impl ColumnKey for total {
            type Table = OrdersMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::NamedSealed for total {}
        impl qbrs_core::row::Named for total {
            type Name = qbrs_core::type_name!('t', 'o', 't', 'a', 'l');
            const NAME: &'static str = "total";
        }
        impl qbrs_core::row::Spelled for total {}
    }

    pub const total: Column<columns::total> = Column::new();
}

#[test]
fn row_number_with_partition_and_order_renders_expected_sql() {
    let q = select((
        employees::department,
        row_number().over(
            window()
                .partition_by(employees::department)
                .order_by(employees::salary.desc()),
        ),
    ))
    .from(employees::Table);

    let (sql, params) = q.to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"employees\".\"department\", row_number() OVER (PARTITION BY \"employees\".\"department\" ORDER BY \"employees\".\"salary\" DESC) FROM \"employees\""
    );
    assert!(params.is_empty());
}

#[test]
fn rank_and_dense_rank_use_their_own_function_names() {
    let q =
        select((rank().over(window().order_by(employees::salary.asc())),)).from(employees::Table);
    assert_eq!(
        q.to_sql(Postgres).0,
        "SELECT rank() OVER (ORDER BY \"employees\".\"salary\" ASC) FROM \"employees\""
    );

    let q = select((dense_rank().over(window().order_by(employees::salary.asc())),))
        .from(employees::Table);
    assert_eq!(
        q.to_sql(Postgres).0,
        "SELECT dense_rank() OVER (ORDER BY \"employees\".\"salary\" ASC) FROM \"employees\""
    );
}

#[test]
fn bare_over_with_no_partition_or_order_renders_empty_parens() {
    let q = select((row_number().over(window()),)).from(employees::Table);
    assert_eq!(
        q.to_sql(Postgres).0,
        "SELECT row_number() OVER () FROM \"employees\""
    );
}

#[test]
fn window_can_be_used_in_a_filter_via_a_derived_table_shape() {
    // `.filter()` doesn't reject a window function expression at the type
    // level (real SQL rejects it at the WHERE-clause position specifically,
    // not because of the expression's shape) — proving it composes with the
    // rest of the expression machinery (e.g. `.gt(..)`) like any other
    // `Expr<Req, S>` once built via `.over()`.
    let q = select((employees::department,))
        .from(employees::Table)
        .having(
            row_number()
                .over(window().order_by(employees::salary.desc()))
                .gt(1i64),
        );
    assert_eq!(
        q.to_sql(Postgres).0,
        "SELECT \"employees\".\"department\" FROM \"employees\" HAVING (row_number() OVER (ORDER BY \"employees\".\"salary\" DESC) > $1)"
    );
}

// Uncomment to eyeball the compile error for partitioning/ordering by a
// column outside the query's scope (confirmed working — kept out of the
// normal test run since it's meant to fail): `orders::total` isn't in scope
// here (no `orders` table joined), so `Req` accumulated by `Window` fails
// the `Superset` check at `.select()`'s terminal `.to_sql(Postgres)`.
//
// #[test]
// fn partitioning_by_an_out_of_scope_column_is_a_compile_error() {
//     let _ = select((row_number().over(window().partition_by(orders::total)),))
//         .from(employees::Table)
//         .to_sql(Postgres); // error[E0277]: the trait bound `Cons<..>: Superset<Cons<OrdersMarker, Nil>, _>` is not satisfied
// }
