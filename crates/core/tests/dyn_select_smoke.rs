use qbrs_core::dialect::Postgres;
use qbrs_core::expr::ExprMethods;
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::{DynSelect, select};

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

fn build(with_orders: bool) -> DynSelect<Postgres, i32> {
    let base = select(users::id).from(users::Table);
    if with_orders {
        base.inner_join(orders::Table, orders::user_id.eq(users::id))
            .erase()
    } else {
        base.erase()
    }
}

#[test]
fn a_query_with_and_without_a_join_erases_to_one_dyn_select_type() {
    let (sql_without, _) = build(false).to_sql(Postgres);
    assert_eq!(sql_without, "SELECT \"users\".\"id\" FROM \"users\"");

    let (sql_with, _) = build(true).to_sql(Postgres);
    assert_eq!(
        sql_with,
        "SELECT \"users\".\"id\" FROM \"users\" INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
}

#[test]
fn an_erased_query_still_counts_its_rows_without_its_page() {
    let (sql, _) = build(true).limit(10).offset(20).count_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT count(*) FROM \"users\" INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
}
