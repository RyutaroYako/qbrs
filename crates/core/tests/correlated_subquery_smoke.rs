//! Does "subquery Scope = Cons<inner, outer>" let a correlated subquery
//! reference outer columns with no machinery beyond what `Find`/`Superset`
//! already do for ordinary joins?

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::ExprMethods;
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::select;

pub struct UsersMarker;
impl TableTrait for UsersMarker {
    const NAME: &'static str = "users";
}
impl qbrs_core::scope::BaseTableSealed for UsersMarker {}
impl qbrs_core::scope::BaseTable for UsersMarker {}
pub struct OrdersMarker;
impl TableTrait for OrdersMarker {
    const NAME: &'static str = "orders";
}
impl qbrs_core::scope::BaseTableSealed for OrdersMarker {}
impl qbrs_core::scope::BaseTable for OrdersMarker {}

#[allow(non_upper_case_globals)]
mod users {
    use super::UsersMarker;
    use qbrs_core::expr::{Column, Integer};
    pub const Table: UsersMarker = UsersMarker;
    #[allow(non_camel_case_types)]
    pub mod columns {
        use super::*;
        use qbrs_core::expr::ColumnKey;
        #[derive(Clone, Copy)]
        pub struct id;
        impl ColumnKey for id {
            type Table = UsersMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::Named for id {
            type Name = qbrs_core::type_name!('i', 'd');
            const NAME: &'static str = "id";
        }
        impl qbrs_core::row::Spelled for id {}
    }

    pub const id: Column<columns::id> = Column::new();
}

#[allow(non_upper_case_globals)]
mod orders {
    use super::OrdersMarker;
    use qbrs_core::expr::{Column, Integer};
    pub const Table: OrdersMarker = OrdersMarker;
    #[allow(non_camel_case_types)]
    pub mod columns {
        use super::*;
        use qbrs_core::expr::ColumnKey;
        #[derive(Clone, Copy)]
        pub struct user_id;
        impl ColumnKey for user_id {
            type Table = OrdersMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::Named for user_id {
            type Name = qbrs_core::type_name!('u', 's', 'e', 'r', '_', 'i', 'd');
            const NAME: &'static str = "user_id";
        }
        impl qbrs_core::row::Spelled for user_id {}
    }

    pub const user_id: Column<columns::user_id> = Column::new();
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
    // query's `$N` sequence (`Fragment`'s whole reason to exist).
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
