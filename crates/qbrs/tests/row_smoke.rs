//! Rows keyed by column: what `.get()` resolves to, what an alias renders
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
/// `GetField`/`Find` bound carries; callers never write it.
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
    // ...and `.into()` is the same conversion.
    let (email, _): (String, Option<i64>) = user_order_row().into();
    assert_eq!(email, "ada@example.com");
}

#[test]
fn an_untupled_selection_stays_a_bare_value() {
    let (sql, _) = select(users::email)
        .from::<Postgres, _>(users::Table)
        .to_sql();
    assert_eq!(sql, "SELECT \"users\".\"email\" FROM \"users\"");
}

qbrs::label!(within_user, overall);

#[test]
fn an_alias_renders_as_and_keys_the_row() {
    let (sql, _) = select((
        users::email,
        row_number()
            .over(window().partition_by(users::id))
            .alias(label::within_user),
        row_number().over(window()).alias(label::overall),
    ))
    .from::<Postgres, _>(users::Table)
    .to_sql();
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
fn an_unaliased_expression_is_keyed_by_the_function_that_made_it() {
    let (sql, _) = select((users::email, count()))
        .from::<Postgres, _>(users::Table)
        .group_by(users::email)
        .to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"email\", (count(*)) FROM \"users\" GROUP BY \"users\".\"email\""
    );

    let row = Row::new(RowCons::<qbrs::expr::Count, _, _>::new(3i64, RowNil));
    assert_eq!(row.count(), &3);
}
