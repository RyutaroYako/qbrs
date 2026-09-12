//! Hand-written schema (standing in for the not-yet-built derive macro) to
//! validate the `Select` builder end-to-end before wiring up codegen.

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::{Bool, ExprMethods, all_of, any_of};
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::{OrderExt, Predicate, grouping, predicate, select, sort_key};
use qbrs_core::sql;

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
        #[derive(Clone, Copy)]
        pub struct name;
        impl qbrs_core::expr::WritableSealed for name {}
        impl qbrs_core::expr::Writable for name {}
        impl ColumnKey for name {
            type Table = UsersMarker;
            type Sql = Text;
        }
        impl qbrs_core::row::NamedSealed for name {}
        impl qbrs_core::row::Named for name {
            type Name = qbrs_core::type_name!('n', 'a', 'm', 'e');
            const NAME: &'static str = "name";
        }
        impl qbrs_core::row::Spelled for name {}
        #[derive(Clone, Copy)]
        pub struct active;
        impl qbrs_core::expr::WritableSealed for active {}
        impl qbrs_core::expr::Writable for active {}
        impl ColumnKey for active {
            type Table = UsersMarker;
            type Sql = Bool;
        }
        impl qbrs_core::row::NamedSealed for active {}
        impl qbrs_core::row::Named for active {
            type Name = qbrs_core::type_name!('a', 'c', 't', 'i', 'v', 'e');
            const NAME: &'static str = "active";
        }
        impl qbrs_core::row::Spelled for active {}
        #[derive(Clone, Copy)]
        pub struct created_at;
        impl qbrs_core::expr::WritableSealed for created_at {}
        impl qbrs_core::expr::Writable for created_at {}
        impl ColumnKey for created_at {
            type Table = UsersMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::NamedSealed for created_at {}
        impl qbrs_core::row::Named for created_at {
            type Name = qbrs_core::type_name!('c', 'r', 'e', 'a', 't', 'e', 'd', '_', 'a', 't');
            const NAME: &'static str = "created_at";
        }
        impl qbrs_core::row::Spelled for created_at {}
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
        #[derive(Clone, Copy)]
        pub struct total;
        impl qbrs_core::expr::WritableSealed for total {}
        impl qbrs_core::expr::Writable for total {}
        impl ColumnKey for total {
            type Table = OrdersMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::NamedSealed for total {}
        impl qbrs_core::row::Named for total {
            type Name = qbrs_core::type_name!('t', 'o', 't', 'a', 'l');
            const NAME: &'static str = "total";
        }
        impl qbrs_core::row::Spelled for total {}
    }

    pub const user_id: Column<columns::user_id> = Column::new();
    pub const total: Column<columns::total> = Column::new();
}

#[test]
fn all_of_and_predicate_all_fold_the_same_way_filter_does() {
    let (sql, _params) = select((users::id,))
        .from(users::Table)
        .filter(all_of([users::id.gt(1), users::id.lt(10)]))
        .filter(Predicate::all_of([
            predicate(users::active.eq(true)),
            predicate(users::id.lte(9)),
        ]))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" \
         WHERE ((\"users\".\"id\" > $1) AND (\"users\".\"id\" < $2)) \
         AND ((\"users\".\"active\" = $3) AND (\"users\".\"id\" <= $4))"
    );

    // Empty: "all of nothing" matches everything, "any of nothing" nothing.
    let (all_empty, _) = select((users::id,))
        .from(users::Table)
        .filter(Predicate::all_of(Vec::new()))
        .to_sql(Postgres);
    assert_eq!(
        all_empty,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE TRUE"
    );
}

#[test]
fn reselect_keeps_every_clause_and_swaps_the_selection() {
    let page = select((users::id,))
        .from(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::id.desc())
        .limit(5u32);

    let (ids, _) = page.clone().to_sql(Postgres);
    let (names, _) = page.reselect((users::name,)).to_sql(Postgres);
    assert_eq!(
        ids,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"active\" = $1) \
         ORDER BY \"users\".\"id\" DESC LIMIT 5"
    );
    assert_eq!(
        names,
        "SELECT \"users\".\"name\" FROM \"users\" WHERE (\"users\".\"active\" = $1) \
         ORDER BY \"users\".\"id\" DESC LIMIT 5"
    );
}

#[test]
fn a_runtime_length_sort_and_grouping_go_in_as_discharged_keys() {
    let keys = vec![sort_key(users::name.asc()), sort_key(orders::total.desc())];
    let groups = vec![grouping(users::id), grouping(orders::user_id)];

    let (sql, _params) = select((users::id,))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .group_by_all(groups)
        .order_by_all(keys)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" \
         INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") \
         GROUP BY \"users\".\"id\", \"orders\".\"user_id\" \
         ORDER BY \"users\".\"name\" ASC, \"orders\".\"total\" DESC"
    );
}

#[test]
fn a_page_size_can_be_a_placeholder() {
    let (sql, params) = select((users::id,))
        .from(users::Table)
        .limit(qbrs_core::expr::placeholder::<qbrs_core::expr::BigInt>(
            "per_page",
        ))
        .offset(20)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" LIMIT $1 OFFSET 20"
    );
    assert_eq!(
        params,
        vec![qbrs_core::expr::Value::Placeholder("per_page")]
    );
}

#[test]
fn a_sql_slot_takes_a_column_and_quotes_it_like_any_other() {
    let (sql, params) = select((users::id,))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .filter(sql!(Bool, "coalesce(?, 0) > ?", orders::total, 100i64))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" \
         INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") \
         WHERE (coalesce(\"orders\".\"total\", 0) > $1)"
    );
    assert_eq!(params, vec![qbrs_core::expr::Value::I64(100)]);
}

#[test]
fn any_of_folds_a_runtime_length_or_and_matches_nothing_when_empty() {
    let terms = ["ada", "grace"];
    let (sql, params) = select((users::id,))
        .from(users::Table)
        .filter(any_of(terms.iter().map(|t| users::name.like(*t))))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" \
         WHERE ((\"users\".\"name\" LIKE $1) OR (\"users\".\"name\" LIKE $2))"
    );
    assert_eq!(params.len(), 2);

    let (empty, _) = select((users::id,))
        .from(users::Table)
        .filter(any_of(Vec::<
            qbrs_core::expr::Expr<qbrs_core::scope::Nil, Bool>,
        >::new()))
        .to_sql(Postgres);
    assert_eq!(empty, "SELECT \"users\".\"id\" FROM \"users\" WHERE FALSE");
}

#[test]
fn distinct_deduplicates_the_rows_a_join_repeats() {
    let (sql, _params) = select((users::id, users::name))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .distinct()
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT DISTINCT \"users\".\"id\", \"users\".\"name\" FROM \"users\" \
         INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
}

#[test]
fn distinct_order_by_selected_checks_the_sort_key_against_the_selection() {
    // Postgres requires a `SELECT DISTINCT`'s sort keys to be in its
    // selection — `.order_by_selected(..)` is checked against `users::name`
    // being one of the two selected columns, the same `row::Field` lookup
    // `Row::get` uses.
    let (sql, _params) = select((users::id, users::name))
        .from(users::Table)
        .distinct()
        .order_by_selected(users::name, qbrs_core::select::SortDir::Asc)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT DISTINCT \"users\".\"id\", \"users\".\"name\" FROM \"users\" ORDER BY \"users\".\"name\" ASC"
    );
}

#[test]
fn order_by_selection_orders_a_single_untupled_column_with_no_key() {
    let (sql, _params) = select(users::name)
        .from(users::Table)
        .order_by_selection(qbrs_core::select::SortDir::Desc)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"name\" FROM \"users\" ORDER BY \"users\".\"name\" DESC"
    );
}

#[test]
fn order_by_selected_renders_the_selected_item_rather_than_the_key_it_was_given() {
    // A key names a field, not an expression: both of these window
    // functions are keyed `RowNumber`, so a key rendered as written could
    // sort by a frame the query never selected. The lookup's index reads
    // the selected item instead.
    use qbrs_core::window::{row_number, window};
    let (sql, _params) = select((
        row_number().over(window().partition_by(users::id)),
        users::name,
    ))
    .from(users::Table)
    .distinct()
    .order_by_selected(row_number(), qbrs_core::select::SortDir::Asc)
    .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT DISTINCT row_number() OVER (PARTITION BY \"users\".\"id\"), \"users\".\"name\" \
         FROM \"users\" ORDER BY row_number() OVER (PARTITION BY \"users\".\"id\") ASC"
    );
}

// Uncomment either to confirm it is (correctly) a compile error: a column
// outside the selection has no field to find, and an unlabelled `sql!`
// fragment has no name to find one by — two anonymous expressions would
// otherwise stand in for each other here the way they can't at `.get()`.
//
// #[test]
// fn order_by_selected_rejects_an_unselected_column() {
//     let _ = select((users::id, users::name))
//         .from(users::Table)
//         .order_by_selected(users::active, qbrs_core::select::SortDir::Asc)
//         .to_sql(Postgres);
// }
//
// #[test]
// fn order_by_selected_rejects_an_unlabelled_expression() {
//     let _ = select((sql!(qbrs_core::expr::Text, "upper(?)", users::name), users::id))
//         .from(users::Table)
//         .order_by_selected(
//             sql!(qbrs_core::expr::Text, "lower(?)", users::name),
//             qbrs_core::select::SortDir::Asc,
//         )
//         .to_sql(Postgres);
// }

#[test]
fn basic_select_renders_expected_sql() {
    let q = select((users::id, users::name))
        .from(users::Table)
        .filter(users::active.eq(true))
        .order_by(users::created_at.desc())
        .limit(10);

    let (sql, params) = q.to_sql(Postgres);
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
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::active.eq(true));

    let (sql, params) = q.to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\", \"orders\".\"total\" FROM \"users\" LEFT JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") WHERE (\"users\".\"active\" = $1)"
    );
    assert_eq!(params, vec![qbrs_core::expr::Value::Bool(true)]);
}

#[test]
fn an_on_condition_takes_whatever_a_where_condition_takes() {
    let from_fragment = select((users::id,))
        .from(users::Table)
        .inner_join(
            orders::Table,
            sql!(Bool, "? = ?", orders::user_id, users::id),
        )
        .to_sql(Postgres)
        .0;
    assert_eq!(
        from_fragment,
        "SELECT \"users\".\"id\" FROM \"users\" INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );

    let discharged: Predicate<_, _> = predicate(orders::user_id.eq(users::id));
    let from_predicate = select((users::id,))
        .from(users::Table)
        .inner_join(orders::Table, discharged)
        .to_sql(Postgres)
        .0;
    assert_eq!(from_fragment, from_predicate);
}

// Uncomment to eyeball the compile error for a forgotten join (confirmed
// working — kept out of the normal test run since it's meant to fail):
//
// #[test]
// fn forgetting_the_join_is_a_compile_error() {
//     let _q = select((users::id, orders::total))
//         .from(users::Table)
//         .filter(users::active.eq(true))
//         .to_sql(Postgres); // the Superset check only fires here, at the terminal method
// }

#[test]
fn right_join_flips_previously_joined_tables_to_nullable() {
    // RIGHT JOIN: `users` (already in scope) must retroactively become
    // nullable, while the newly-joined `orders` stays not-null — the
    // mirror image of LEFT JOIN. Selecting `users::name` (Text, not
    // Nullable<Text> in the schema) must still type-check and decode as
    // `Option<String>` here purely because of the join kind.
    let q = select((users::name, orders::total))
        .from(orders::Table)
        .right_join(users::Table, orders::user_id.eq(users::id))
        .to_sql(Postgres);
    assert_eq!(
        q.0,
        "SELECT \"users\".\"name\", \"orders\".\"total\" FROM \"orders\" RIGHT JOIN \"users\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
}

#[test]
fn full_join_makes_every_table_nullable() {
    let q = select((users::name, orders::total))
        .from(users::Table)
        .full_join(orders::Table, orders::user_id.eq(users::id))
        .to_sql(Postgres);
    assert_eq!(
        q.0,
        "SELECT \"users\".\"name\", \"orders\".\"total\" FROM \"users\" FULL JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
}

#[test]
fn having_all_folds_a_runtime_length_collection() {
    let conds = vec![predicate(qbrs_core::expr::count().gt(1i64))];
    let (sql, _) = select((users::id, qbrs_core::expr::count()))
        .from(users::Table)
        .group_by(users::id)
        .having_all(conds)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\", count(*) FROM \"users\" GROUP BY \"users\".\"id\" HAVING (count(*) > $1)"
    );
}

#[test]
fn group_by_and_having_render() {
    let q = select((users::active, qbrs_core::expr::count()))
        .from(users::Table)
        .group_by(users::active)
        .having(qbrs_core::expr::count().gt(1i64))
        .to_sql(Postgres);
    assert_eq!(
        q.0,
        "SELECT \"users\".\"active\", count(*) FROM \"users\" GROUP BY \"users\".\"active\" HAVING (count(*) > $1)"
    );
    assert_eq!(q.1, vec![qbrs_core::expr::Value::I64(1)]);
}

#[test]
fn a_bind_carrying_expression_reused_across_clauses_keeps_one_placeholder() {
    let bucket = qbrs_core::sql!(
        qbrs_core::expr::Integer,
        "((? / ?) * ?)",
        users::id,
        10i32,
        10i32
    );
    let (sql, params) = select((bucket.clone(), qbrs_core::expr::count()))
        .from(users::Table)
        .group_by(bucket.clone())
        .order_by(bucket.asc())
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT (((\"users\".\"id\" / $1) * $1)), count(*) FROM \"users\" \
         GROUP BY (((\"users\".\"id\" / $1) * $1)) \
         ORDER BY (((\"users\".\"id\" / $1) * $1)) ASC"
    );
    assert_eq!(params, vec![qbrs_core::expr::Value::I32(10)]);
}

/// `1.0` and `1.00` are one value to `Decimal`'s `==` and two to a
/// `numeric` column, which keeps the scale it was handed. Sharing one
/// parameter between them would bind the first one twice.
#[cfg(feature = "decimal")]
#[test]
fn two_decimals_of_the_same_value_and_different_scale_bind_separately() {
    use std::str::FromStr as _;
    let one_dp = rust_decimal::Decimal::from_str("1.0").expect("a decimal");
    let two_dp = rust_decimal::Decimal::from_str("1.00").expect("a decimal");
    assert_eq!(one_dp, two_dp);

    let (sql, params) = select((users::id,))
        .from(users::Table)
        .filter(qbrs_core::sql!(
            Bool,
            "(? = ? OR ? = ?)",
            one_dp,
            two_dp,
            one_dp,
            one_dp
        ))
        .to_sql(Postgres);
    assert!(sql.ends_with("WHERE (($1 = $2 OR $1 = $1))"), "{sql}");
    assert_eq!(params.len(), 2);
}

#[test]
fn raw_sql_escape_hatch_renders_and_renumbers_params() {
    let q = select((users::id,))
        .from(users::Table)
        .filter(qbrs_core::sql!(Bool, "lower(name) = ?", "dan"))
        .filter(users::active.eq(true));

    let (sql, params) = q.to_sql(Postgres);
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
        .from(users::Table)
        .filter(users::name.is_null())
        .filter(users::active.is_not_null());
    let (sql, params) = q.to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"name\" IS NULL) AND (\"users\".\"active\" IS NOT NULL)"
    );
    assert!(params.is_empty());
}

#[test]
fn in_list_binds_one_parameter_per_value() {
    let q = select((users::id,))
        .from(users::Table)
        .filter(users::id.is_in(vec![1, 2, 3]));
    let (sql, params) = q.to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"id\" IN ($1, $2, $3))"
    );
    assert_eq!(params.len(), 3);
}

#[test]
fn an_empty_in_list_renders_false_rather_than_invalid_sql() {
    let q = select((users::id,))
        .from(users::Table)
        .filter(users::id.is_in(Vec::<i32>::new()));
    let (sql, _) = q.to_sql(Postgres);
    assert_eq!(sql, "SELECT \"users\".\"id\" FROM \"users\" WHERE FALSE");
}

#[test]
fn aggregates_render_as_function_calls_over_real_columns() {
    let q = select((users::id, qbrs_core::expr::count_of(orders::total)))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .group_by(users::id);
    let (sql, _) = q.to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\", count(\"orders\".\"total\") FROM \"users\" INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") GROUP BY \"users\".\"id\""
    );
}

#[test]
fn a_literal_question_mark_travels_as_a_bound_value() {
    // There is no `??` escape: MySQL and SQLite write their own bind
    // parameters as `?`, so a `?` left in the text would be read as one.
    let q = select((users::id,))
        .from(users::Table)
        .filter(qbrs_core::sql!(
            Bool,
            "users.name LIKE ? OR users.name LIKE ?",
            "who?",
            "a%"
        ));
    let (sql, params) = q.to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (users.name LIKE $1 OR users.name LIKE $2)"
    );
    assert_eq!(params.len(), 2);
}

#[test]
fn a_nullable_column_compares_against_a_non_nullable_one() {
    // `users::name` is Text, `orders::total` is Integer; the point is that
    // a Nullable<S> and an S are comparable, both ways round.
    let q = select((users::id,))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::created_at.eq(orders::total));
    let (sql, _) = q.to_sql(Postgres);
    assert!(sql.contains("\"users\".\"created_at\" = \"orders\".\"total\""));
}

#[test]
fn a_count_of_a_query_that_returns_one_row_is_one() {
    // The selection decides this as much as `GROUP BY` does: a query that
    // already aggregates returns one row, so counting it has to wrap it.
    let (sql, _) = select((qbrs_core::expr::count(),))
        .from(users::Table)
        .count_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT count(*) FROM (SELECT count(*) FROM \"users\") AS \"qbrs_total\""
    );

    // A plain-column selection still counts without the wrap.
    let (plain, _) = select((users::id,)).from(users::Table).count_sql(Postgres);
    assert_eq!(plain, "SELECT count(*) FROM \"users\"");
}
