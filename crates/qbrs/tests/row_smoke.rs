// Aliasing each row type to quiet `clippy::type_complexity` would only rename
// the key list it spells, which is the shape these tests pin down.
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

fn decoded_user_order_row()
-> Row<RowCons<users::columns::email, String, RowCons<orders::columns::total, Option<i64>, RowNil>>>
{
    Row::new(RowCons::new(
        "ada@example.com".to_string(),
        RowCons::new(Some(1000i64), RowNil),
    ))
}

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
fn a_sql_fragment_with_a_column_slot_decodes_as_its_declared_type_without_decodes_as() {
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
    let row = decoded_user_order_row();
    assert_eq!(row.get(users::email), "ada@example.com");
    assert_eq!(row.get(orders::total), &Some(1000));
}

#[test]
fn a_nested_selection_produces_the_same_sql_and_row_type_as_its_flat_spelling() {
    const CORE: (Column<users::columns::id>, Column<users::columns::email>) =
        (users::id, users::email);

    fn same_row_type<D, Scope, A, B, IdxA, IdxB>(_: &Select<D, Scope, A>, _: &Select<D, Scope, B>)
    where
        A: Selection<Scope, IdxA>,
        B: Selection<Scope, IdxB, Output = A::Output>,
    {
    }

    let nested = select((CORE, users::display_name)).from(users::Table);
    let spelled_out = select((users::id, users::email, users::display_name)).from(users::Table);
    same_row_type(&nested, &spelled_out);
    assert_eq!(nested.to_sql(Postgres).0, spelled_out.to_sql(Postgres).0);
}

#[test]
fn two_text_columns_of_one_table_are_read_by_key_whatever_their_position() {
    let row = Row::new(RowCons::<users::columns::display_name, _, _>::new(
        Some("Ada".to_string()),
        RowCons::<users::columns::email, _, _>::new("ada@example.com".to_string(), RowNil),
    ));
    assert_eq!(row.get(users::email), "ada@example.com");
    assert_eq!(row.get(users::display_name), &Some("Ada".to_string()));
}

#[test]
fn derive_generates_a_per_column_accessor_on_a_decoded_row() {
    let row = decoded_user_order_row();
    assert_eq!(row.email(), "ada@example.com");
    assert_eq!(row.total(), &Some(1000));
}

fn masked<Idx, R: users::HasEmail<Idx, Value = String>>(row: &R) -> String {
    format!("{}***", &row.email()[..1])
}

#[test]
fn a_helper_can_require_one_column_and_ignore_the_rest() {
    assert_eq!(masked(&decoded_user_order_row()), "a***");
}

#[test]
fn into_tuple_recovers_the_positional_view() {
    let (email, total): (String, Option<i64>) = decoded_user_order_row().into_tuple();
    assert_eq!(email, "ada@example.com");
    assert_eq!(total, Some(1000));
}

#[test]
fn into_tuple_reaches_the_thirty_second_field_which_is_the_arity_cap() {
    type Cell<Tail> = RowCons<users::columns::id, i64, Tail>;
    type Cells2<Tail> = Cell<Cell<Tail>>;
    type Cells4<Tail> = Cells2<Cells2<Tail>>;
    type Cells8<Tail> = Cells4<Cells4<Tail>>;
    type Cells16<Tail> = Cells8<Cells8<Tail>>;

    fn cell<Tail>(tail: Tail) -> Cell<Tail> {
        RowCons::new(0, tail)
    }
    fn cells2<Tail>(tail: Tail) -> Cells2<Tail> {
        cell(cell(tail))
    }
    fn cells4<Tail>(tail: Tail) -> Cells4<Tail> {
        cells2(cells2(tail))
    }
    fn cells8<Tail>(tail: Tail) -> Cells8<Tail> {
        cells4(cells4(tail))
    }
    fn cells16<Tail>(tail: Tail) -> Cells16<Tail> {
        cells8(cells8(tail))
    }

    let thirty_second = RowCons::<users::columns::email, _, _>::new("last".to_string(), RowNil);
    let fields = Row::new(cells16(cells8(cells4(cells2(cell(thirty_second)))))).into_tuple();

    assert_eq!(fields.0, 0);
    assert_eq!(fields.31, "last");
}

#[test]
fn an_untupled_selection_decodes_to_a_bare_value_rather_than_a_row() {
    let query = select(users::email).from(users::Table);
    assert_eq!(
        query.to_sql(Postgres).0,
        "SELECT \"users\".\"email\" FROM \"users\""
    );
    decodes_to(&query, "ada@example.com".to_string());
}

qbrs::label!(within_user, overall);

#[test]
fn a_label_renders_as_an_as_alias_and_keys_the_row() {
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

#[derive(Debug, PartialEq, FromRow)]
struct UserSummary {
    email: String,
    display_name: Option<String>,
    total: Option<i64>,
}

#[test]
fn from_row_matches_fields_by_name_ignoring_order_and_extras() {
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
fn from_row_matches_a_declared_label_by_its_name() {
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
    let row = decoded_user_order_row();
    let (email, row) = row.take(users::email);
    let (total, _) = row.take(orders::total);
    assert_eq!(email, "ada@example.com");
    assert_eq!(total, Some(1000));
}

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
fn a_row_can_be_walked_by_column_name_from_a_downstream_trait() {
    let mut out = Vec::new();
    decoded_user_order_row().fields().to_pairs(&mut out);
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
fn a_count_drops_the_ordering_and_paging_of_the_page_it_counts() {
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
fn an_expression_over_a_column_is_selectable_under_a_label_once_decodes_as_names_its_type() {
    qbrs::label!(flagged);
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

#[test]
fn a_window_function_is_read_back_by_the_bare_function_without_its_window() {
    use qbrs::window::{row_number, window};

    let (sql, _) = select((
        users::email,
        row_number().over(window().order_by(users::id.asc())),
    ))
    .from(users::Table)
    .to_sql(Postgres);
    assert!(sql.contains("row_number() OVER (ORDER BY \"users\".\"id\" ASC)"));

    fn read(r: Row<RowCons<qbrs::window::RowNumber, i64, RowNil>>) -> i64 {
        *r.get(row_number())
    }
    let _ = read;
}
