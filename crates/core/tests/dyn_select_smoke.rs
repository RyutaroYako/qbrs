//! Proves `DynSelect` actually solves the one case the design plan
//! identified as a mathematical impossibility for static typing: a single
//! static type cannot mean "joined" in one branch and "not joined" in
//! another, so unifying two differently-shaped queries needs *some*
//! erasure — this checks that `.erase()` is narrow enough to still catch
//! real mistakes (a forgotten join) while being just permissive enough to
//! let the two branches unify.

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::ExprMethods;
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::{DynSelect, select};

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

fn build(with_orders: bool) -> DynSelect<Postgres, (i32,)> {
    let base = select((users::id,)).from::<Postgres, _>(users::Table);
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
