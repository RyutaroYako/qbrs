//! `x IN (<subquery>)` / `x NOT IN (<subquery>)`: does `Select::contains`
//! render correctly, stay dialect-pinned the way `EXISTS` does, and type-check
//! the outer expression against the subquery's single selected column the
//! same way `.eq(..)` type-checks two columns?

use qbrs_core::dialect::{MySql, Postgres};
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
        impl qbrs_core::expr::WritableSealed for id {}
        impl qbrs_core::expr::Writable for id {}
        impl ColumnKey for id {
            type Table = UsersMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::NamedSealed for id {}
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
    use qbrs_core::expr::{BigInt, Column};
    pub const Table: OrdersMarker = OrdersMarker;
    #[allow(non_camel_case_types)]
    pub mod columns {
        use super::*;
        use qbrs_core::expr::ColumnKey;
        #[derive(Clone, Copy)]
        pub struct user_id;
        impl qbrs_core::expr::WritableSealed for user_id {}
        impl qbrs_core::expr::Writable for user_id {}
        impl ColumnKey for user_id {
            type Table = OrdersMarker;
            type Sql = BigInt;
        }
        impl qbrs_core::row::NamedSealed for user_id {}
        impl qbrs_core::row::Named for user_id {
            type Name = qbrs_core::type_name!('u', 's', 'e', 'r', '_', 'i', 'd');
            const NAME: &'static str = "user_id";
        }
        impl qbrs_core::row::Spelled for user_id {}
    }

    pub const user_id: Column<columns::user_id> = Column::new();
}

#[test]
fn an_uncorrelated_in_subquery_renders_and_typechecks() {
    // `orders::user_id` is `BigInt` and `users::id` is `Integer`, comparable
    // across widths exactly like `.eq(..)`. That proves `contains` reuses the
    // same `Comparable` check rather than requiring identical SQL types.
    let subquery = select(orders::user_id).from(orders::Table);
    let outer = select((users::id,)).from(users::Table);
    let cond = subquery.contains(users::id);

    let (sql, params) = outer.filter(cond).to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"id\" IN (SELECT \"orders\".\"user_id\" FROM \"orders\"))"
    );
    assert!(params.is_empty());
}

#[test]
fn not_contains_renders_not_in() {
    let subquery = select(orders::user_id).from(orders::Table);
    let outer = select((users::id,)).from(users::Table);
    let cond = subquery.not_contains(users::id);

    let (sql, _params) = outer.filter(cond).to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"id\" NOT IN (SELECT \"orders\".\"user_id\" FROM \"orders\"))"
    );
}

#[test]
fn a_correlated_in_subquery_references_the_outer_column() {
    // `.contains()` is built from `Outer::Tables` the same way `.exists()`
    // is, so a subquery started via `.correlated(..)` can reference the
    // outer column in its own `.filter()` while `contains`'s `lhs` also
    // reaches into the outer scope. Both go through the one flat cons-list,
    // with no correlation-specific machinery.
    let outer = select((users::id,)).from(users::Table);
    let subquery = outer.correlated(orders::Table, orders::user_id);
    let cond = subquery
        .filter(orders::user_id.eq(users::id))
        .contains(users::id);

    let (sql, params) = outer.filter(cond).to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"id\" IN (SELECT \"orders\".\"user_id\" FROM \"orders\" WHERE (\"orders\".\"user_id\" = \"users\".\"id\")))"
    );
    assert!(params.is_empty());
}

#[test]
fn an_in_subquery_is_a_condition_of_its_own_dialect() {
    // Pinned rather than dialect-agnostic, for the same reason `EXISTS` is:
    // the subquery was checked against its dialect's capabilities, so it can
    // only be filtered onto a statement of the same one.
    let subquery = select(orders::user_id).from(orders::Table);
    let (sql, _) = select((users::id,))
        .from(users::Table)
        .filter(subquery.contains(users::id))
        .to_sql(MySql);
    assert_eq!(
        sql,
        "SELECT `users`.`id` FROM `users` WHERE (`users`.`id` IN (SELECT `orders`.`user_id` FROM `orders`))"
    );
}
