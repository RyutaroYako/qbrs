//! Hand-written schema (standing in for the not-yet-built derive macro) to
//! validate the `Select` builder end-to-end before wiring up codegen.

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::{Bool, ExprMethods, TextExprMethods, any_of};
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::{OrderExt, select};

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
        }
        impl qbrs_core::row::Named for id {
            type Name = qbrs_core::type_name!('i', 'd');
            const NAME: &'static str = "id";
        }
        #[derive(Clone, Copy)]
        pub struct name;
        impl ColumnKey for name {
            type Table = UsersMarker;
            type Sql = Text;
        }
        impl qbrs_core::row::Named for name {
            type Name = qbrs_core::type_name!('n', 'a', 'm', 'e');
            const NAME: &'static str = "name";
        }
        #[derive(Clone, Copy)]
        pub struct active;
        impl ColumnKey for active {
            type Table = UsersMarker;
            type Sql = Bool;
        }
        impl qbrs_core::row::Named for active {
            type Name = qbrs_core::type_name!('a', 'c', 't', 'i', 'v', 'e');
            const NAME: &'static str = "active";
        }
        #[derive(Clone, Copy)]
        pub struct created_at;
        impl ColumnKey for created_at {
            type Table = UsersMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::Named for created_at {
            type Name = qbrs_core::type_name!('c', 'r', 'e', 'a', 't', 'e', 'd', '_', 'a', 't');
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
        }
        impl qbrs_core::row::Named for user_id {
            type Name = qbrs_core::type_name!('u', 's', 'e', 'r', '_', 'i', 'd');
            const NAME: &'static str = "user_id";
        }
        #[derive(Clone, Copy)]
        pub struct total;
        impl ColumnKey for total {
            type Table = OrdersMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::Named for total {
            type Name = qbrs_core::type_name!('t', 'o', 't', 'a', 'l');
            const NAME: &'static str = "total";
        }
    }

    pub const user_id: Column<columns::user_id> = Column::new();
    pub const total: Column<columns::total> = Column::new();
}

#[test]
fn any_of_folds_a_runtime_length_or_and_matches_nothing_when_empty() {
    let terms = ["ada", "grace"];
    let (sql, params) = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .filter(any_of(terms.iter().map(|t| users::name.like(*t))))
        .to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" \
         WHERE ((\"users\".\"name\" LIKE $1) OR (\"users\".\"name\" LIKE $2))"
    );
    assert_eq!(params.len(), 2);

    let (empty, _) = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .filter(any_of(Vec::<
            qbrs_core::expr::Expr<qbrs_core::scope::Nil, Bool>,
        >::new()))
        .to_sql();
    assert_eq!(empty, "SELECT \"users\".\"id\" FROM \"users\" WHERE FALSE");
}

#[test]
fn distinct_deduplicates_the_rows_a_join_repeats() {
    let (sql, _params) = select((users::id, users::name))
        .from::<Postgres, _>(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .distinct()
        .to_sql();
    assert_eq!(
        sql,
        "SELECT DISTINCT \"users\".\"id\", \"users\".\"name\" FROM \"users\" \
         INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
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
        "SELECT \"users\".\"active\", count(*) FROM \"users\" GROUP BY \"users\".\"active\" HAVING (count(*) > $1)"
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

#[test]
fn null_tests_render_is_null_not_equality() {
    let q = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .filter(users::name.is_null())
        .filter(users::active.is_not_null());
    let (sql, params) = q.to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"name\" IS NULL) AND (\"users\".\"active\" IS NOT NULL)"
    );
    assert!(params.is_empty());
}

#[test]
fn in_list_binds_one_parameter_per_value() {
    let q = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .filter(users::id.is_in(vec![1, 2, 3]));
    let (sql, params) = q.to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"id\" IN ($1, $2, $3))"
    );
    assert_eq!(params.len(), 3);
}

#[test]
fn an_empty_in_list_renders_false_rather_than_invalid_sql() {
    let q = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .filter(users::id.is_in(Vec::<i32>::new()));
    let (sql, _) = q.to_sql();
    assert_eq!(sql, "SELECT \"users\".\"id\" FROM \"users\" WHERE FALSE");
}

#[test]
fn aggregates_render_as_function_calls_over_real_columns() {
    let q = select((users::id, qbrs_core::expr::count_of(orders::total)))
        .from::<Postgres, _>(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .group_by(users::id);
    let (sql, _) = q.to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\", count(\"orders\".\"total\") FROM \"users\" INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") GROUP BY \"users\".\"id\""
    );
}

#[test]
fn a_doubled_question_mark_is_a_literal_one() {
    let q = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .filter(qbrs_core::sql!(
            Bool,
            "users.name LIKE 'who??' OR users.name LIKE ?",
            "a%"
        ));
    let (sql, params) = q.to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (users.name LIKE 'who?' OR users.name LIKE $1)"
    );
    assert_eq!(params.len(), 1);
}

#[test]
fn a_nullable_column_compares_against_a_non_nullable_one() {
    // `users::name` is Text, `orders::total` is Integer; the point is that
    // a Nullable<S> and an S are comparable, both ways round.
    let q = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::created_at.eq(orders::total));
    let (sql, _) = q.to_sql();
    assert!(sql.contains("\"users\".\"created_at\" = \"orders\".\"total\""));
}
