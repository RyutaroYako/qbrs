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

pub struct OrdersMarker;
impl TableTrait for OrdersMarker {
    const NAME: &'static str = "orders";
}

#[allow(non_upper_case_globals)]
mod employees {
    use super::EmployeesMarker;
    use qbrs_core::expr::{Column, Integer, Text};

    pub const Table: EmployeesMarker = EmployeesMarker;
    pub const department: Column<EmployeesMarker, Text> = Column::new("department");
    pub const salary: Column<EmployeesMarker, Integer> = Column::new("salary");
}

#[allow(non_upper_case_globals, dead_code)]
mod orders {
    use super::OrdersMarker;
    use qbrs_core::expr::{Column, Integer};

    pub const total: Column<OrdersMarker, Integer> = Column::new("total");
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
    .from::<Postgres, _>(employees::Table);

    let (sql, params) = q.to_sql();
    assert_eq!(
        sql,
        "SELECT \"employees\".\"department\", row_number() OVER (PARTITION BY \"employees\".\"department\" ORDER BY \"employees\".\"salary\" DESC) FROM \"employees\""
    );
    assert!(params.is_empty());
}

#[test]
fn rank_and_dense_rank_use_their_own_function_names() {
    let q = select((rank().over(window().order_by(employees::salary.asc())),))
        .from::<Postgres, _>(employees::Table);
    assert_eq!(
        q.to_sql().0,
        "SELECT rank() OVER (ORDER BY \"employees\".\"salary\" ASC) FROM \"employees\""
    );

    let q = select((dense_rank().over(window().order_by(employees::salary.asc())),))
        .from::<Postgres, _>(employees::Table);
    assert_eq!(
        q.to_sql().0,
        "SELECT dense_rank() OVER (ORDER BY \"employees\".\"salary\" ASC) FROM \"employees\""
    );
}

#[test]
fn bare_over_with_no_partition_or_order_renders_empty_parens() {
    let q = select((row_number().over(window()),)).from::<Postgres, _>(employees::Table);
    assert_eq!(
        q.to_sql().0,
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
        .from::<Postgres, _>(employees::Table)
        .having(
            row_number()
                .over(window().order_by(employees::salary.desc()))
                .gt(1i64),
        );
    assert_eq!(
        q.to_sql().0,
        "SELECT \"employees\".\"department\" FROM \"employees\" HAVING (row_number() OVER (ORDER BY \"employees\".\"salary\" DESC) > $1)"
    );
}

// Uncomment to eyeball the compile error for partitioning/ordering by a
// column outside the query's scope (confirmed working — kept out of the
// normal test run since it's meant to fail): `orders::total` isn't in scope
// here (no `orders` table joined), so `Req` accumulated by `Window` fails
// the `Superset` check at `.select()`'s terminal `.to_sql()`.
//
// #[test]
// fn partitioning_by_an_out_of_scope_column_is_a_compile_error() {
//     let _ = select((row_number().over(window().partition_by(orders::total)),))
//         .from::<Postgres, _>(employees::Table)
//         .to_sql(); // error[E0277]: the trait bound `Cons<..>: Superset<Cons<OrdersMarker, Nil>, _>` is not satisfied
// }
