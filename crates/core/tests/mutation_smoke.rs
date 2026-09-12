//! Hand-written `InsertRow`/`UpdateRow` impls (standing in for the derive
//! macro) to validate INSERT/UPDATE/DELETE end-to-end.

use qbrs_core::delete::delete;
use qbrs_core::dialect::Postgres;
use qbrs_core::expr::Integer;
use qbrs_core::expr::{ExprMethods, Value};
use qbrs_core::insert::{
    ConflictUpdate, Defaultable, InsertRow, InsertValue, excluded, insert, partial_index,
};
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::statement::Statement;
use qbrs_core::update::{Assignments, NothingToSet, UpdateRow, update};

pub struct UsersMarker;
impl TableTrait for UsersMarker {
    const NAME: &'static str = "users";
}
impl qbrs_core::scope::BaseTableSealed for UsersMarker {}
impl qbrs_core::scope::BaseTable for UsersMarker {}

#[allow(non_upper_case_globals, dead_code)]
mod users {
    use super::UsersMarker;
    use qbrs_core::expr::{Column, Integer, Text};

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
        pub struct email;
        impl qbrs_core::expr::WritableSealed for email {}
        impl qbrs_core::expr::Writable for email {}
        impl ColumnKey for email {
            type Table = UsersMarker;
            type Sql = Text;
        }
        impl qbrs_core::row::NamedSealed for email {}
        impl qbrs_core::row::Named for email {
            type Name = qbrs_core::type_name!('e', 'm', 'a', 'i', 'l');
            const NAME: &'static str = "email";
        }
        impl qbrs_core::row::Spelled for email {}
        #[derive(Clone, Copy)]
        pub struct display_name;
        impl qbrs_core::expr::WritableSealed for display_name {}
        impl qbrs_core::expr::Writable for display_name {}
        impl ColumnKey for display_name {
            type Table = UsersMarker;
            type Sql = Text;
        }
        impl qbrs_core::row::NamedSealed for display_name {}
        impl qbrs_core::row::Named for display_name {
            type Name =
                qbrs_core::type_name!('d', 'i', 's', 'p', 'l', 'a', 'y', '_', 'n', 'a', 'm', 'e');
            const NAME: &'static str = "display_name";
        }
        impl qbrs_core::row::Spelled for display_name {}

        #[derive(Clone, Copy)]
        pub struct created_at;
        impl qbrs_core::expr::WritableSealed for created_at {}
        impl qbrs_core::expr::Writable for created_at {}
        impl ColumnKey for created_at {
            type Table = UsersMarker;
            type Sql = Text;
        }
        impl qbrs_core::row::NamedSealed for created_at {}
        impl qbrs_core::row::Named for created_at {
            type Name = qbrs_core::type_name!('c', 'r', 'e', 'a', 't', 'e', 'd', '_', 'a', 't');
            const NAME: &'static str = "created_at";
        }
        impl qbrs_core::row::Spelled for created_at {}
    }

    pub const id: Column<columns::id> = Column::new();
    pub const email: Column<columns::email> = Column::new();
    pub const display_name: Column<columns::display_name> = Column::new();
}

// What #[derive(Table)] will generate for:
//   #[column(primary_key, generated)] id: i64
//   #[column(not_null)]               email: String
//   #[column(nullable)]               display_name: Option<String>
//   #[column(not_null, default)]      created_at: String   (kept as String to avoid a chrono dep here)
struct UsersInsert {
    email: String,
    display_name: Option<String>,
    created_at: Defaultable<String>,
}

/// Stands in for the type-state builder `#[derive(Table)]` emits; this file
/// hand-writes the schema so `qbrs-core` can be tested without the macros.
struct UsersInsertBuilder;

impl UsersInsert {
    fn builder() -> UsersInsertBuilder {
        UsersInsertBuilder
    }
}

impl UsersInsertBuilder {
    fn email(self, email: impl Into<String>) -> UsersInsert {
        UsersInsert {
            email: email.into(),
            display_name: None,
            created_at: Defaultable::Default,
        }
    }
}

impl UsersInsert {
    fn build(self) -> Self {
        self
    }
}

impl qbrs_core::insert::InsertRowSealed for UsersInsert {}
impl qbrs_core::insert::InsertableSealed for UsersInsert {}
impl qbrs_core::insert::Insertable for UsersInsert {}

impl InsertRow for UsersInsert {
    type Table = UsersMarker;
    type Values = qbrs_core::row::RowCons<
        users::columns::email,
        InsertValue,
        qbrs_core::row::RowCons<
            users::columns::display_name,
            InsertValue,
            qbrs_core::row::RowCons<
                users::columns::created_at,
                InsertValue,
                qbrs_core::row::RowNil,
            >,
        >,
    >;

    fn into_values(self) -> Self::Values {
        qbrs_core::row::RowCons::new(
            InsertValue::Value(self.email.into()),
            qbrs_core::row::RowCons::new(
                match self.display_name {
                    Some(v) => InsertValue::Value(v.into()),
                    None => InsertValue::Value(Value::NullText),
                },
                qbrs_core::row::RowCons::new(self.created_at.into(), qbrs_core::row::RowNil),
            ),
        )
    }
}

#[derive(Default)]
struct UsersUpdate {
    email: Option<String>,
    display_name: Option<Option<String>>,
}

impl qbrs_core::update::UpdateRowSealed for UsersUpdate {}

impl UpdateRow for UsersUpdate {
    type Table = UsersMarker;
    fn sets(self) -> Vec<(&'static str, Value)> {
        let mut v = Vec::new();
        if let Some(email) = self.email {
            v.push(("email", email.into()));
        }
        if let Some(display_name) = self.display_name {
            v.push((
                "display_name",
                match display_name {
                    Some(s) => s.into(),
                    None => Value::NullText,
                },
            ));
        }
        v
    }
}

#[test]
fn a_set_list_can_be_expressions_alone() {
    let (sql, params) = update(users::Table)
        .set(Assignments::set_to(
            users::email,
            qbrs_core::sql!(qbrs_core::expr::Text, "lower(?)", users::email),
        ))
        .filter(users::id.eq(1))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"email\" = (lower(\"users\".\"email\")) WHERE (\"users\".\"id\" = $1)"
    );
    assert_eq!(params.len(), 1);
}

#[test]
fn set_to_assigns_an_expression_and_appends_to_the_row() {
    let (sql, params) = update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                email: Some("new@example.com".into()),
                display_name: None,
            })
            .expect("email is set"),
        )
        .set_to(
            users::display_name,
            qbrs_core::sql!(qbrs_core::expr::Text, "upper(?)", users::email),
        )
        .filter(users::id.eq(1))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"email\" = $1, \"display_name\" = (upper(\"users\".\"email\")) \
         WHERE (\"users\".\"id\" = $2)"
    );
    assert_eq!(params.len(), 2);
}

#[test]
fn insert_omits_default_as_the_default_keyword() {
    let (sql, params) = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"created_at\") VALUES ($1, $2, DEFAULT)"
    );
    assert_eq!(
        params,
        vec![Value::Text("a@example.com".into()), Value::NullText]
    );
}

#[test]
fn insert_bulk_and_returning() {
    let (sql, _params) = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .values(UsersInsert::builder().email("b@example.com").build())
        .returning(users::id)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"created_at\") VALUES ($1, $2, DEFAULT), ($3, $2, DEFAULT) RETURNING \"users\".\"id\""
    );
}

#[test]
fn values_all_appends_to_a_statement_that_already_has_a_row() {
    let (sql, params) = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .values_all(
            ["b@example.com", "c@example.com"]
                .into_iter()
                .map(|email| UsersInsert::builder().email(email).build()),
        )
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"created_at\") VALUES ($1, $2, DEFAULT), ($3, $2, DEFAULT), ($4, $2, DEFAULT)"
    );
    assert_eq!(params.len(), 4);
}

#[test]
fn an_update_that_sets_nothing_is_an_error_not_a_panic() {
    let nothing = Assignments::<UsersMarker>::from_row(UsersUpdate::default());
    assert!(matches!(nothing, Err(NothingToSet)));
}

#[test]
fn update_only_touches_set_fields() {
    let (sql, params) = update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                email: Some("new@example.com".into()),
                display_name: None,
            })
            .expect("email is set"),
        )
        .filter(users::id.eq(1))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"email\" = $1 WHERE (\"users\".\"id\" = $2)"
    );
    assert_eq!(
        params,
        vec![Value::Text("new@example.com".into()), Value::I32(1)]
    );
}

#[test]
fn upsert_do_nothing_renders_conflict_target() {
    let (sql, params) = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .on_conflict_do_nothing(users::email)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"created_at\") VALUES ($1, $2, DEFAULT) ON CONFLICT (\"email\") DO NOTHING"
    );
    assert_eq!(
        params,
        vec![Value::Text("a@example.com".into()), Value::NullText]
    );
}

#[test]
fn an_upsert_assigns_the_row_the_insert_proposed() {
    let (sql, params) = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .on_conflict_do_update(
            users::email,
            ConflictUpdate::set_to(users::display_name, excluded(users::display_name)),
        )
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"created_at\") VALUES ($1, $2, DEFAULT) \
         ON CONFLICT (\"email\") DO UPDATE SET \"display_name\" = excluded.\"display_name\""
    );
    assert_eq!(params.len(), 2);
}

/// The proposed row is an ordinary expression over the target's columns, so
/// the two compose — which is the assignment a counter upsert is written
/// with, and the one passing the same Rust value to both halves can't say.
#[test]
fn the_proposed_row_composes_with_the_conflicting_one() {
    let (sql, _) = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .on_conflict_do_update(
            users::email,
            ConflictUpdate::set_to(
                users::id,
                qbrs_core::sql!(Integer, "(? + ?)", users::id, excluded(users::id)),
            ),
        )
        .to_sql(Postgres);
    assert!(
        sql.ends_with("DO UPDATE SET \"id\" = ((\"users\".\"id\" + excluded.\"id\"))"),
        "{sql}"
    );
}

#[test]
fn a_partial_index_target_repeats_the_index_predicate() {
    let (sql, params) = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .on_conflict_do_update(
            partial_index(users::email, users::display_name.is_null()),
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("A".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
        )
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"created_at\") VALUES ($1, $2, DEFAULT) \
         ON CONFLICT (\"email\") WHERE (\"users\".\"display_name\" IS NULL) DO UPDATE SET \"display_name\" = $3"
    );
    assert_eq!(params.len(), 3);
}

#[test]
fn a_partial_index_target_of_several_columns_renders_do_nothing_after_the_predicate() {
    let (sql, _) = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .on_conflict_do_nothing(partial_index(
            (users::email, users::id),
            users::display_name.is_null(),
        ))
        .to_sql(Postgres);
    assert!(
        sql.ends_with(
            "ON CONFLICT (\"email\", \"id\") WHERE (\"users\".\"display_name\" IS NULL) DO NOTHING"
        ),
        "{sql}"
    );
}

#[test]
fn upsert_do_update_reuses_update_row_and_supports_returning() {
    let (sql, params) = insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .on_conflict_do_update(
            users::email,
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("A".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
        )
        .returning(users::id)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"created_at\") VALUES ($1, $2, DEFAULT) \
         ON CONFLICT (\"email\") DO UPDATE SET \"display_name\" = $3 RETURNING \"users\".\"id\""
    );
    assert_eq!(
        params,
        vec![
            Value::Text("a@example.com".into()),
            Value::NullText,
            Value::Text("A".into()),
        ]
    );
}

#[test]
fn delete_renders_where() {
    let (sql, params) = delete(users::Table)
        .filter(users::id.eq(1))
        .to_sql(Postgres);
    assert_eq!(sql, "DELETE FROM \"users\" WHERE (\"users\".\"id\" = $1)");
    assert_eq!(params, vec![Value::I32(1)]);
}
