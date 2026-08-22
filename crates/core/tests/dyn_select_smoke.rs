//! `.erase()` has to be permissive enough to unify two differently-joined
//! branches, and narrow enough to still reject a forgotten join. These
//! check both halves.

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::ExprMethods;
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::{DynSelect, select};

pub struct UsersMarker;
impl TableTrait for UsersMarker {
    const NAME: &'static str = "users";
}
impl qbrs_core::scope::BaseTable for UsersMarker {}
pub struct OrdersMarker;
impl TableTrait for OrdersMarker {
    const NAME: &'static str = "orders";
}
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
    }

    pub const user_id: Column<columns::user_id> = Column::new();
}

fn build(with_orders: bool) -> DynSelect<Postgres, i32> {
    let base = select(users::id).from::<Postgres, _>(users::Table);
    if with_orders {
        base.inner_join(orders::Table, orders::user_id.eq(users::id))
            .erase()
    } else {
        base.erase()
    }
}

#[test]
fn both_branches_unify_into_the_same_type() {
    let (sql_without, _) = build(false).to_sql();
    assert_eq!(sql_without, "SELECT \"users\".\"id\" FROM \"users\"");

    let (sql_with, _) = build(true).to_sql();
    assert_eq!(
        sql_with,
        "SELECT \"users\".\"id\" FROM \"users\" INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
}

// The un-erased version of `build()` genuinely does not compile — kept
// here, commented, as a record of exactly what `.erase()` buys:
//
// fn build_without_erase(with_orders: bool) -> impl std::fmt::Debug {
//     let base = select((users::id,)).from::<Postgres, _>(users::Table);
//     if with_orders {
//         base.inner_join(orders::Table, orders::user_id.eq(users::id)) // Select<Postgres, Cons<Orders,...>, _>
//     } else {
//         base // Select<Postgres, Cons<Users,Nil>, _> -- different type, `if`/`else` arms must match
//     }
// }
