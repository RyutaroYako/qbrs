//! Rows keyed by column: what `.get()` resolves to, what a label renders
//! as, and what survives a column being added to a selection.

// Naming a row type spells out its key list, which is what
// `clippy::type_complexity` counts — the same reason `qbrs-core` allows it
// crate-wide.
#![allow(clippy::type_complexity)]

use qbrs::prelude::*;
use qbrs::row::{Row, RowCons, RowNil};

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
    #[column(default)]
    active: bool,
}

#[derive(Table)]
#[table(name = "orders")]
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

/// Stands in for a decoded row; `.load()` builds the same shape.
fn user_order_row()
-> Row<RowCons<users::columns::email, String, RowCons<orders::columns::total, Option<i64>, RowNil>>>
{
    Row::new(RowCons::new(
        "ada@example.com".to_string(),
        RowCons::new(Some(1000i64), RowNil),
    ))
}

/// Pins a query's decoded row type to a value written out by hand: passing
/// a row of any other shape is a compile error.
fn decodes_to<D, Scope, Sel, Idx>(_query: &Select<D, Scope, Sel>, _row: Sel::Output)
where
    Sel: Selection<Scope, Idx>,
{
}

#[test]
fn all_selects_every_column_and_takes_its_nullability_from_the_join() {
    let query = select((users::email, orders::All))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id));

    let (sql, _params) = query.to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"email\", \"orders\".\"id\", \"orders\".\"user_id\", \"orders\".\"total\" \
         FROM \"users\" LEFT JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")"
    );

    decodes_to(
        &query,
        Row::new(RowCons::<users::columns::email, _, _>::new(
            "ada@example.com".to_string(),
            RowCons::<orders::columns::id, Option<i64>, _>::new(
                None,
                RowCons::<orders::columns::user_id, Option<i64>, _>::new(
                    None,
                    RowCons::<orders::columns::total, Option<i64>, _>::new(None, RowNil),
                ),
            ),
        )),
    );
}

#[test]
fn a_sql_fragment_decodes_as_the_type_it_declares() {
    // No `.decodes_as(..)` restating it: the author wrote the type in the
    // `sql!` itself, and a slot holding a column doesn't change that.
    label!(biggest);
    let query = select((
        users::email,
        sql!(Nullable<BigInt>, "max(?)", orders::total).label(label::biggest),
    ))
    .from(users::Table)
    .inner_join(orders::Table, orders::user_id.eq(users::id))
    .group_by(users::email);

    let (sql, _params) = query.to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"email\", (max(\"orders\".\"total\")) AS \"biggest\" FROM \"users\" \
         INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") \
         GROUP BY \"users\".\"email\""
    );

    decodes_to(
        &query,
        Row::new(RowCons::<users::columns::email, _, _>::new(
            "ada@example.com".to_string(),
            RowCons::<label::biggest, Option<i64>, _>::new(Some(10), RowNil),
        )),
    );
}

#[test]
fn a_field_is_read_by_the_value_that_selected_it() {
    let row = user_order_row();
    assert_eq!(row.get(users::email), "ada@example.com");
    assert_eq!(row.get(orders::total), &Some(1000));
}

#[test]
fn two_same_typed_columns_of_one_table_stay_distinct() {
    // `users::email` and `users::display_name` are both text columns, so a
    // positional tuple would let them be transposed silently.
    let row = Row::new(RowCons::<users::columns::display_name, _, _>::new(
        Some("Ada".to_string()),
        RowCons::<users::columns::email, _, _>::new("ada@example.com".to_string(), RowNil),
    ));
    assert_eq!(row.get(users::email), "ada@example.com");
    assert_eq!(row.get(users::display_name), &Some("Ada".to_string()));
}

#[test]
fn derive_generates_a_per_column_accessor() {
    let row = user_order_row();
    assert_eq!(row.email(), "ada@example.com");
    assert_eq!(row.total(), &Some(1000));
}

/// Width subtyping: this accepts any row that carries `users::email`,
/// whatever else it holds. `Idx` is the inferred lookup index every
/// `Field`/`Find` bound carries; callers never write it.
fn masked<Idx, R: users::HasEmail<Idx, Value = String>>(row: &R) -> String {
    format!("{}***", &row.email()[..1])
}

#[test]
fn a_helper_can_require_one_column_and_ignore_the_rest() {
    assert_eq!(masked(&user_order_row()), "a***");
}

#[test]
fn into_tuple_recovers_the_positional_view() {
    let (email, total): (String, Option<i64>) = user_order_row().into_tuple();
    assert_eq!(email, "ada@example.com");
    assert_eq!(total, Some(1000));
}

#[test]
fn an_untupled_selection_stays_a_bare_value() {
    let (sql, _) = select(users::email).from(users::Table).to_sql(Postgres);
    assert_eq!(sql, "SELECT \"users\".\"email\" FROM \"users\"");
}

qbrs::label!(within_user, overall);

#[test]
fn a_label_renders_as_and_keys_the_row() {
    let (sql, _) = select((
        users::email,
        row_number()
            .over(window().partition_by(users::id))
            .label(label::within_user),
        row_number().over(window()).label(label::overall),
    ))
    .from(users::Table)
    .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"email\", row_number() OVER (PARTITION BY \"users\".\"id\") AS \"within_user\", row_number() OVER () AS \"overall\" FROM \"users\""
    );

    let row = Row::new(RowCons::<label::within_user, _, _>::new(
        1i64,
        RowCons::<label::overall, _, _>::new(7i64, RowNil),
    ));
    assert_eq!(row.get(label::within_user), &1);
    assert_eq!(row.overall(), &7);
}

#[test]
fn an_unlabelled_expression_is_keyed_by_the_function_that_made_it() {
    let (sql, _) = select((users::email, count()))
        .from(users::Table)
        .group_by(users::email)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"email\", count(*) FROM \"users\" GROUP BY \"users\".\"email\""
    );

    let row = Row::new(RowCons::<qbrs::expr::Count, _, _>::new(3i64, RowNil));
    assert_eq!(row.count(), &3);
}

/// A plain domain struct: no column paths, no query shape, nothing that ties
/// it to the query that fills it.
#[derive(Debug, PartialEq, FromRow)]
struct UserSummary {
    email: String,
    display_name: Option<String>,
    total: Option<i64>,
}

#[test]
fn from_row_matches_fields_by_name_ignoring_order_and_extras() {
    // Selected as (id, total, email, display_name); the struct declares a
    // different order and doesn't want `id` at all.
    let row = Row::new(RowCons::<users::columns::id, _, _>::new(
        7i64,
        RowCons::<orders::columns::total, _, _>::new(
            Some(2500i64),
            RowCons::<users::columns::email, _, _>::new(
                "ada@example.com".to_string(),
                RowCons::<users::columns::display_name, _, _>::new(Some("Ada".to_string()), RowNil),
            ),
        ),
    ));

    let summary: UserSummary = row.into_struct();
    assert_eq!(
        summary,
        UserSummary {
            email: "ada@example.com".to_string(),
            display_name: Some("Ada".to_string()),
            total: Some(2500),
        }
    );
}

#[derive(Debug, PartialEq, FromRow)]
struct Ranked {
    email: String,
    within_user: i64,
}

#[test]
fn a_declared_label_is_matched_by_its_name() {
    let row = Row::new(RowCons::<users::columns::email, _, _>::new(
        "ada@example.com".to_string(),
        RowCons::<label::within_user, _, _>::new(1i64, RowNil),
    ));
    assert_eq!(
        row.into_struct::<Ranked, _>(),
        Ranked {
            email: "ada@example.com".to_string(),
            within_user: 1
        }
    );
}

#[test]
fn take_moves_one_field_and_keeps_the_rest() {
    let row = user_order_row();
    let (email, row) = row.take(users::email);
    let (total, _) = row.take(orders::total);
    assert_eq!(email, "ada@example.com");
    assert_eq!(total, Some(1000));
}

/// A row carries names, so another crate can walk it under its own bounds —
/// `serde::Serialize`, `Display`, whatever — which is what `qbrs-core`
/// cannot offer itself, having no dependencies.
trait ToPairs {
    fn to_pairs(&self, out: &mut Vec<(&'static str, String)>);
}
impl ToPairs for RowNil {
    fn to_pairs(&self, _out: &mut Vec<(&'static str, String)>) {}
}
impl<K: qbrs::row::Named, V: std::fmt::Debug, T: ToPairs> ToPairs for RowCons<K, V, T> {
    fn to_pairs(&self, out: &mut Vec<(&'static str, String)>) {
        out.push((K::NAME, format!("{:?}", self.value())));
        self.tail().to_pairs(out);
    }
}

#[test]
fn a_row_can_be_walked_by_a_downstream_trait() {
    let mut out = Vec::new();
    user_order_row().fields().to_pairs(&mut out);
    assert_eq!(
        out,
        vec![
            ("email", "\"ada@example.com\"".to_string()),
            ("total", "Some(1000)".to_string()),
        ]
    );
}

#[test]
fn a_grouped_count_counts_groups_not_the_first_group() {
    let (sql, _) = select((users::id, count()))
        .from(users::Table)
        .group_by(users::id)
        .count_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT count(*) FROM (SELECT \"users\".\"id\", count(*) FROM \"users\" \
         GROUP BY \"users\".\"id\") AS \"qbrs_total\""
    );
}

#[test]
fn counting_a_distinct_query_counts_its_distinct_rows() {
    let (sql, _params) = select((users::id,))
        .from(users::Table)
        .inner_join(orders::Table, orders::user_id.eq(users::id))
        .distinct()
        .count_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT count(*) FROM (SELECT DISTINCT \"users\".\"id\" FROM \"users\" \
         INNER JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\")) AS \"qbrs_total\""
    );
}

#[test]
fn a_count_drops_the_paging_the_page_needed() {
    let base = select((users::email,))
        .from(users::Table)
        .filter(users::active.eq(true));
    let page = base
        .clone()
        .order_by(users::id.asc())
        .limit(20u32)
        .offset(40u32);
    assert_eq!(
        page.count_sql(Postgres).0,
        "SELECT count(*) FROM \"users\" WHERE (\"users\".\"active\" = $1)"
    );
}

#[test]
fn a_named_expression_over_a_column_states_what_it_decodes_to() {
    qbrs::label!(flagged);
    // `.decodes_as()` is the difference between a type the builder guessed from
    // whatever built the expression and one the caller stands behind.
    let (sql, _) = select((
        users::id,
        users::active
            .eq(true)
            .decodes_as::<qbrs::expr::Bool>()
            .label(label::flagged),
    ))
    .from(users::Table)
    .to_sql(Postgres);
    assert!(sql.contains("AS \"flagged\""), "{sql}");
}
