use std::marker::PhantomData;

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::{Bool, ExprMethods, all_of, any_of};
use qbrs_core::row::{Row, RowCons, RowNil};
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::{
    OrderExt, Predicate, Select, Selection, grouping, predicate, select, sort_key,
};
use qbrs_core::sql;

// Written out by hand rather than with `#[derive(Table)]`, so that these tests
// exercise `qbrs-core` with no dependency on `qbrs-macros`.
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
    use qbrs_core::scope::Nullable;

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
        #[derive(Clone, Copy)]
        pub struct discount;
        impl qbrs_core::expr::WritableSealed for discount {}
        impl qbrs_core::expr::Writable for discount {}
        impl ColumnKey for discount {
            type Table = OrdersMarker;
            type Sql = Nullable<Integer>;
        }
        impl qbrs_core::row::NamedSealed for discount {}
        impl qbrs_core::row::Named for discount {
            type Name = qbrs_core::type_name!('d', 'i', 's', 'c', 'o', 'u', 'n', 't');
            const NAME: &'static str = "discount";
        }
        impl qbrs_core::row::Spelled for discount {}
    }

    pub const user_id: Column<columns::user_id> = Column::new();
    pub const total: Column<columns::total> = Column::new();
    pub const discount: Column<columns::discount> = Column::new();
}

fn decoded_row<D, Scope, Sel: Selection<Scope, Idx>, Idx>(
    _query: &Select<D, Scope, Sel>,
) -> PhantomData<Sel::Output> {
    PhantomData
}

type IdAndTotal<Total> =
    Row<RowCons<users::columns::id, i32, RowCons<orders::columns::total, Total, RowNil>>>;
type NameAndTotal<Name, Total> =
    Row<RowCons<users::columns::name, Name, RowCons<orders::columns::total, Total, RowNil>>>;

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
}

#[test]
fn predicate_all_of_an_empty_list_renders_true() {
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
fn a_placeholder_limit_binds_while_a_literal_offset_is_written_out() {
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
fn distinct_renders_select_distinct_ahead_of_the_selection() {
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
fn order_by_selected_sorts_a_distinct_query_by_one_of_its_selected_columns() {
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
    // The `ORDER BY` repeats the selected `OVER (..)` rather than the bare
    // `row_number()` passed in: both are keyed `RowNumber`, so a key rendered
    // as written could sort by a frame the query never selected.
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

#[test]
fn a_named_selection_is_reused_bare_and_as_one_element_of_a_longer_or_nested_list() {
    const CORE: (
        qbrs_core::expr::Column<users::columns::id>,
        qbrs_core::expr::Column<users::columns::name>,
    ) = (users::id, users::name);

    let (listing, _) = select(CORE).from(users::Table).to_sql(Postgres);
    assert_eq!(
        listing,
        "SELECT \"users\".\"id\", \"users\".\"name\" FROM \"users\""
    );

    let (detail, _) = select((CORE, users::active, qbrs_core::expr::count()))
        .from(users::Table)
        .to_sql(Postgres);
    assert_eq!(
        detail,
        "SELECT \"users\".\"id\", \"users\".\"name\", \"users\".\"active\", count(*) FROM \"users\""
    );

    let (nested, _) = select(((CORE, users::active), users::created_at))
        .from(users::Table)
        .to_sql(Postgres);
    assert_eq!(
        nested,
        "SELECT \"users\".\"id\", \"users\".\"name\", \"users\".\"active\", \"users\".\"created_at\" FROM \"users\""
    );
}

#[test]
fn where_order_by_and_limit_render_in_keyword_order_and_only_the_filter_binds() {
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
fn left_join_makes_the_joined_table_nullable_and_leaves_the_others_alone() {
    let q = select((users::id, orders::total))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::active.eq(true));
    let _: PhantomData<IdAndTotal<Option<i32>>> = decoded_row(&q);

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

#[test]
fn right_join_flips_previously_joined_tables_to_nullable() {
    let q = select((users::name, orders::total))
        .from(orders::Table)
        .right_join(users::Table, orders::user_id.eq(users::id));
    let _: PhantomData<NameAndTotal<String, Option<i32>>> = decoded_row(&q);

    assert_eq!(
        q.to_sql(Postgres).0,
        "SELECT \"users\".\"name\", \"orders\".\"total\" FROM \"orders\" RIGHT JOIN \"users\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );
}

#[test]
fn full_join_makes_every_table_nullable() {
    let q = select((users::name, orders::total))
        .from(users::Table)
        .full_join(orders::Table, orders::user_id.eq(users::id));
    let _: PhantomData<NameAndTotal<Option<String>, Option<i32>>> = decoded_row(&q);

    assert_eq!(
        q.to_sql(Postgres).0,
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
fn having_follows_group_by_and_binds_its_value() {
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

#[cfg(feature = "decimal")]
#[test]
fn two_decimals_of_the_same_value_and_different_scale_bind_separately() {
    // Two parameters rather than one shared through `Decimal`'s `==`, which
    // equates the two: a `numeric` column keeps the scale it is handed.
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

#[cfg(feature = "json")]
#[test]
fn two_json_documents_that_compare_equal_and_render_apart_bind_separately() {
    // Two parameters rather than one shared through `serde_json::Value`'s
    // `==`, which equates the two: a column stores the sign it is handed.
    let positive: serde_json::Value = serde_json::json!({ "x": 0.0 });
    let negative: serde_json::Value = serde_json::json!({ "x": -0.0 });
    assert_eq!(positive, negative);

    let (sql, params) = select((users::id,))
        .from(users::Table)
        .filter(qbrs_core::sql!(
            Bool,
            "(? = ? OR ? = ?)",
            positive.clone(),
            negative,
            positive.clone(),
            positive
        ))
        .to_sql(Postgres);
    assert!(sql.ends_with("WHERE (($1 = $2 OR $1 = $1))"), "{sql}");
    assert_eq!(params.len(), 2);
}

#[test]
fn a_sql_fragment_value_is_numbered_in_sequence_with_the_filters_around_it() {
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
fn count_of_a_column_renders_count_over_that_qualified_column() {
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
fn string_agg_puts_its_separator_where_each_dialect_takes_one() {
    use qbrs_core::dialect::{MySql, Sqlite};
    use qbrs_core::expr::string_agg;
    let (sql, params) = select((users::id, string_agg(users::name, ", ")))
        .from::<Postgres, _>(users::Table)
        .group_by(users::id)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        r#"SELECT "users"."id", string_agg("users"."name", $1) FROM "users" GROUP BY "users"."id""#
    );
    assert_eq!(params, vec![qbrs_core::expr::Value::Text(", ".into())]);

    let (sql, params) = select((users::id, string_agg(users::name, ", ")))
        .from::<Sqlite, _>(users::Table)
        .group_by(users::id)
        .to_sql(Sqlite);
    assert_eq!(
        sql,
        r#"SELECT "users"."id", group_concat("users"."name", ?) FROM "users" GROUP BY "users"."id""#
    );
    assert_eq!(params, vec![qbrs_core::expr::Value::Text(", ".into())]);

    // Written out rather than bound like the other two: MySQL's grammar
    // rejects a parameter after `SEPARATOR`.
    let (sql, params) = select((users::id, string_agg(users::name, ", ")))
        .from::<MySql, _>(users::Table)
        .group_by(users::id)
        .to_sql(MySql);
    assert_eq!(
        sql,
        "SELECT `users`.`id`, group_concat(`users`.`name` SEPARATOR ', ') FROM `users` GROUP BY `users`.`id`"
    );
    assert!(params.is_empty());
}

#[test]
fn a_separator_written_out_for_mysql_is_escaped_the_way_mysql_reads_one() {
    // A string assertion rather than an executed statement: nothing in this
    // workspace runs MySQL.
    use qbrs_core::dialect::MySql;
    use qbrs_core::expr::string_agg;
    assert_eq!(
        select((string_agg(users::name, r"'\"),))
            .from::<MySql, _>(users::Table)
            .to_sql(MySql)
            .0,
        r"SELECT group_concat(`users`.`name` SEPARATOR '''\\') FROM `users`"
    );
}

#[test]
fn a_selection_naming_no_table_renders_a_select_with_no_from() {
    let (sql, params) = select((
        qbrs_core::sql!(qbrs_core::expr::Text, "current_setting('TimeZone')"),
        qbrs_core::sql!(qbrs_core::expr::BigInt, "(? + ?)", 2i64, 3i64),
    ))
    .to_sql(Postgres);
    assert_eq!(sql, "SELECT (current_setting('TimeZone')), (($1 + $2))");
    assert_eq!(
        params,
        vec![
            qbrs_core::expr::Value::I64(2),
            qbrs_core::expr::Value::I64(3)
        ]
    );
}

#[test]
fn a_literal_question_mark_travels_as_a_bound_value() {
    // Bound rather than escaped as `??` in the text: MySQL and SQLite read
    // any `?` left in the text as a bind parameter of their own.
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
fn a_nullable_column_compares_against_a_non_nullable_one_both_ways_round() {
    let q = select((users::id,))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::created_at.eq(orders::discount))
        .filter(orders::discount.eq(users::created_at));
    let (sql, _) = q.to_sql(Postgres);
    assert!(
        sql.ends_with(
            "WHERE (\"users\".\"created_at\" = \"orders\".\"discount\") \
             AND (\"orders\".\"discount\" = \"users\".\"created_at\")"
        ),
        "{sql}"
    );
}

#[test]
fn counting_a_query_that_only_aggregates_wraps_it_so_its_one_row_counts_as_one() {
    let (sql, _) = select((qbrs_core::expr::count(),))
        .from(users::Table)
        .count_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT count(*) FROM (SELECT count(*) FROM \"users\") AS \"qbrs_total\""
    );
}

#[test]
fn counting_a_plain_column_query_replaces_its_selection_with_count_and_no_wrap() {
    let (plain, _) = select((users::id,)).from(users::Table).count_sql(Postgres);
    assert_eq!(plain, "SELECT count(*) FROM \"users\"");
}
