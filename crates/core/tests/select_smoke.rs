//! Hand-written schema (standing in for the not-yet-built derive macro) to
//! validate the `Select` builder end-to-end before wiring up codegen.

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::{Bool, ExprMethods};
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::{OrderExt, select};

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
    use qbrs_core::expr::{Bool, Column, Integer, Text};

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
            const NAME: &'static str = "id";
        }
        #[derive(Clone, Copy)]
        pub struct name;
        impl ColumnKey for name {
            type Table = UsersMarker;
            type Sql = Text;
            const NAME: &'static str = "name";
        }
        #[derive(Clone, Copy)]
        pub struct active;
        impl ColumnKey for active {
            type Table = UsersMarker;
            type Sql = Bool;
            const NAME: &'static str = "active";
        }
        #[derive(Clone, Copy)]
        pub struct created_at;
        impl ColumnKey for created_at {
            type Table = UsersMarker;
            type Sql = Integer;
            const NAME: &'static str = "created_at";
        }
    }

    pub const id: Column<columns::id> = Column::new();
    pub const name: Column<columns::name> = Column::new();
    pub const active: Column<columns::active> = Column::new();
    pub const created_at: Column<columns::created_at> = Column::new();
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
            const NAME: &'static str = "user_id";
        }
        #[derive(Clone, Copy)]
        pub struct total;
        impl ColumnKey for total {
            type Table = OrdersMarker;
            type Sql = Integer;
            const NAME: &'static str = "total";
        }
    }

    pub const user_id: Column<columns::user_id> = Column::new();
    pub const total: Column<columns::total> = Column::new();
}

#[test]
fn basic_select_renders_expected_sql() {
    let q = select((users::id, users::name))
        .from::<Postgres, _>(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::created_at.desc())
        .limit(10);

    let (sql, params) = q.to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\", \"users\".\"name\" FROM \"users\" WHERE (\"users\".\"active\" = $1) ORDER BY \"users\".\"created_at\" DESC LIMIT 10"
    );
    assert_eq!(params, vec![qbrs_core::expr::Value::Bool(true)]);
}

#[test]
fn left_join_renders_and_typechecks() {
    // The join condition and the select-list both reference `orders`
    // *after* it's been joined — this is the case that would fail to
    // compile (Superset unsatisfied) if the join were forgotten.
    let q = select((users::id, orders::total))
        .from::<Postgres, _>(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::active.eq(true));

    let (sql, params) = q.to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\", \"orders\".\"total\" FROM \"users\" LEFT JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") WHERE (\"users\".\"active\" = $1)"
    );
    assert_eq!(params, vec![qbrs_core::expr::Value::Bool(true)]);
}

// Uncomment to eyeball the compile error for a forgotten join (confirmed
// working — kept out of the normal test run since it's meant to fail):
//
// #[test]
// fn forgetting_the_join_is_a_compile_error() {
//     let _q = select((users::id, orders::total))
//         .from::<Postgres, _>(users::Table)
//         .filter(users::active.eq(true))
//         .to_sql(); // the Superset check only fires here, at the terminal method
// }

#[test]
fn right_join_flips_previously_joined_tables_to_nullable() {
    // RIGHT JOIN: `users` (already in scope) must retroactively become
    // nullable, while the newly-joined `orders` stays not-null — the
    // mirror image of LEFT JOIN. Selecting `users::name` (Text, not
    // Nullable<Text> in the schema) must still type-check and decode as
    // `Option<String>` here purely because of the join kind.
    let q = select((users::name, orders::total))
        .from::<Postgres, _>(orders::Table)
        .right_join(users::Table, orders::user_id.eq(users::id))
        .to_sql();
    assert_eq!(
        q.0,
        "SELECT \"users\".\"name\", \"orders\".\"total\" FROM \"orders\" RIGHT JOIN \"users\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
}

#[test]
fn full_join_makes_every_table_nullable() {
    let q = select((users::name, orders::total))
        .from::<Postgres, _>(users::Table)
        .full_join(orders::Table, orders::user_id.eq(users::id))
        .to_sql();
    assert_eq!(
        q.0,
        "SELECT \"users\".\"name\", \"orders\".\"total\" FROM \"users\" FULL JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
}

#[test]
fn group_by_and_having_render() {
    let q = select((users::active, qbrs_core::expr::count()))
        .from::<Postgres, _>(users::Table)
        .group_by(users::active)
        .having(qbrs_core::expr::count().gt(1i64))
        .to_sql();
    assert_eq!(
        q.0,
        "SELECT \"users\".\"active\", (count(*)) FROM \"users\" GROUP BY \"users\".\"active\" HAVING ((count(*)) > $1)"
    );
    assert_eq!(q.1, vec![qbrs_core::expr::Value::I64(1)]);
}

#[test]
fn raw_sql_escape_hatch_renders_and_renumbers_params() {
    let q = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .filter(qbrs_core::sql!(Bool, "lower(name) = ?", "dan"))
        .filter(users::active.eq(true));

    let (sql, params) = q.to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (lower(name) = $1) AND (\"users\".\"active\" = $2)"
    );
    assert_eq!(
        params,
        vec![
            qbrs_core::expr::Value::Text("dan".into()),
            qbrs_core::expr::Value::Bool(true),
        ]
    );
}
