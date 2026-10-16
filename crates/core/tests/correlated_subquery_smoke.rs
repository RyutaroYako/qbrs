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
    use qbrs_core::expr::{Column, Integer};
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
            type Sql = Integer;
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
fn a_correlated_exists_filters_on_its_own_table_and_the_outer_one_in_one_expression() {
    let outer = select((users::id,)).from(users::Table);
    let subquery = outer.correlated(orders::Table, (orders::user_id,));
    let cond = subquery.filter(orders::user_id.eq(users::id)).exists();

    let (sql, params) = outer.filter(cond).to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (EXISTS (SELECT \"orders\".\"user_id\" FROM \"orders\" WHERE (\"orders\".\"user_id\" = \"users\".\"id\")))"
    );
    assert!(params.is_empty());
}

#[test]
fn a_correlated_subquerys_placeholders_are_numbered_after_the_outer_querys() {
    let outer = select((users::id,))
        .from(users::Table)
        .filter(users::id.gt(0));
    let subquery = outer.correlated(orders::Table, (orders::user_id,));
    let cond = subquery
        .filter(orders::user_id.eq(users::id))
        .filter(orders::user_id.lt(5))
        .exists();

    let (sql, params) = outer.filter(cond).to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"id\" > $1) AND (EXISTS (SELECT \"orders\".\"user_id\" FROM \"orders\" WHERE (\"orders\".\"user_id\" = \"users\".\"id\") AND (\"orders\".\"user_id\" < $2)))"
    );
    assert_eq!(
        params,
        vec![
            qbrs_core::expr::Value::I32(0),
            qbrs_core::expr::Value::I32(5)
        ]
    );
}

#[test]
fn an_exists_is_a_condition_of_its_own_dialect() {
    let inner = select((orders::user_id,)).from(orders::Table);
    let (sql, _) = select((users::id,))
        .from(users::Table)
        .filter(inner.exists())
        .to_sql(MySql);
    assert_eq!(
        sql,
        "SELECT `users`.`id` FROM `users` WHERE (EXISTS (SELECT `orders`.`user_id` FROM `orders`))"
    );
}
