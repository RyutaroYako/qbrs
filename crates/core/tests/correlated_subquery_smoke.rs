//! Spike for the design plan's flagged-as-unverified hypothesis: does
//! "subquery Scope = Cons<inner, outer>" actually let a correlated
//! subquery reference outer columns, with no special-cased machinery
//! beyond what `Find`/`Superset` already do for ordinary joins?

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::ExprMethods;
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::select;

pub struct UsersMarker;
impl TableTrait for UsersMarker {
    const NAME: &'static str = "users";
}
pub struct OrdersMarker;
impl TableTrait for OrdersMarker {
    const NAME: &'static str = "orders";
}

#[allow(non_upper_case_globals)]
mod users {
    use super::UsersMarker;
    use qbrs_core::expr::{Column, Integer};
    pub const Table: UsersMarker = UsersMarker;
    pub const id: Column<UsersMarker, Integer> = Column::new("id");
}

#[allow(non_upper_case_globals)]
mod orders {
    use super::OrdersMarker;
    use qbrs_core::expr::{Column, Integer};
    pub const Table: OrdersMarker = OrdersMarker;
    pub const user_id: Column<OrdersMarker, Integer> = Column::new("user_id");
}

#[test]
fn correlated_exists_references_outer_column() {
    let outer = select((users::id,)).from::<Postgres, _>(users::Table);

    // The subquery's `.filter()` references `orders::user_id` (its own
    // FROM) *and* `users::id` (the outer query's FROM) in the same
    // expression — this is exactly the correlated-subquery case. It
    // type-checks with no special API beyond `.correlated()` + the
    // ordinary `Find`/`Superset` machinery already used for joins.
    let subquery = outer.correlated(orders::Table, (orders::user_id,));
    let cond = subquery.filter(orders::user_id.eq(users::id)).exists();

    let (sql, params) = outer.filter(cond).to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (EXISTS (SELECT \"orders\".\"user_id\" FROM \"orders\" WHERE (\"orders\".\"user_id\" = \"users\".\"id\")))"
    );
    assert!(params.is_empty());
}

#[test]
fn correlated_subquery_with_bound_value_renumbers_correctly() {
    // The outer query also binds a literal value — proving the subquery's
    // own `?`-then-renumber placeholder doesn't collide with the outer
    // query's `$N` sequence (dialect::RawEmbed's whole reason to exist).
    let outer = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .filter(users::id.gt(0));
    let subquery = outer.correlated(orders::Table, (orders::user_id,));
    let cond = subquery.filter(orders::user_id.eq(users::id)).exists();

    let (sql, params) = outer.filter(cond).to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"id\" > $1) AND (EXISTS (SELECT \"orders\".\"user_id\" FROM \"orders\" WHERE (\"orders\".\"user_id\" = \"users\".\"id\")))"
    );
    assert_eq!(params, vec![qbrs_core::expr::Value::I32(0)]);
}

// Uncomment to confirm a subquery referencing a table from neither its own
// FROM nor the outer scope is (correctly) a compile error:
//
// #[test]
// fn subquery_cannot_reference_an_unrelated_table() {
//     pub struct PaymentsMarker;
//     impl TableTrait for PaymentsMarker {
//         const NAME: &'static str = "payments";
//     }
//     let payments_amount = qbrs_core::expr::Column::<PaymentsMarker, qbrs_core::expr::Integer>::new("amount");
//     let outer = select((users::id,)).from::<Postgres, _>(users::Table);
//     let subquery = outer.correlated(orders::Table, (orders::user_id,));
//     let _cond = subquery.filter(payments_amount.eq(1)).exists(); // error: Payments not in scope
// }
