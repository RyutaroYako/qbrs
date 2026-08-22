//! Hand-written `InsertRow`/`UpdateRow` impls (standing in for the derive
//! macro) to validate INSERT/UPDATE/DELETE end-to-end.

use qbrs_core::delete::delete;
use qbrs_core::dialect::Postgres;
use qbrs_core::expr::{ExprMethods, Value};
use qbrs_core::insert::{Defaultable, InsertRow, InsertValue, insert};
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::update::{NothingToSet, UpdateRow, update};

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
        impl ColumnKey for id {
            type Table = UsersMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::Named for id {
            type Name = qbrs_core::type_name!('i', 'd');
            const NAME: &'static str = "id";
        }
        #[derive(Clone, Copy)]
        pub struct email;
        impl ColumnKey for email {
            type Table = UsersMarker;
            type Sql = Text;
        }
        impl qbrs_core::row::Named for email {
            type Name = qbrs_core::type_name!('e', 'm', 'a', 'i', 'l');
            const NAME: &'static str = "email";
        }
        #[derive(Clone, Copy)]
        pub struct display_name;
        impl ColumnKey for display_name {
            type Table = UsersMarker;
            type Sql = Text;
        }
        impl qbrs_core::row::Named for display_name {
            type Name =
                qbrs_core::type_name!('d', 'i', 's', 'p', 'l', 'a', 'y', '_', 'n', 'a', 'm', 'e');
            const NAME: &'static str = "display_name";
        }
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

impl InsertRow for UsersInsert {
    type Table = UsersMarker;
    const COLUMNS: &'static [&'static str] = &["email", "display_name", "created_at"];
    fn into_values(self) -> Vec<InsertValue> {
        vec![
            InsertValue::Value(self.email.into()),
            match self.display_name {
                Some(v) => InsertValue::Value(v.into()),
                None => InsertValue::Value(Value::NullText),
            },
            self.created_at.into(),
        ]
    }
}

#[derive(Default)]
struct UsersUpdate {
    email: Option<String>,
    display_name: Option<Option<String>>,
}

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
fn insert_omits_default_as_the_default_keyword() {
    let (sql, params) = insert::<Postgres, _>(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .to_sql();
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
    let (sql, _params) = insert::<Postgres, _>(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .values(UsersInsert::builder().email("b@example.com").build())
        .returning(users::id)
        .to_sql();
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"created_at\") VALUES ($1, $2, DEFAULT), ($3, $4, DEFAULT) RETURNING \"users\".\"id\""
    );
}

#[test]
fn an_update_that_sets_nothing_is_an_error_not_a_panic() {
    let nothing = update::<Postgres, _>(users::Table).set(UsersUpdate::default());
    assert!(matches!(nothing, Err(NothingToSet)));

    let nothing = insert::<Postgres, _>(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .on_conflict_do_update(users::email, UsersUpdate::default());
    assert!(matches!(nothing, Err(NothingToSet)));
}

#[test]
fn update_only_touches_set_fields() {
    let (sql, params) = update::<Postgres, _>(users::Table)
        .set(UsersUpdate {
            email: Some("new@example.com".into()),
            display_name: None,
        })
        .expect("email is set")
        .filter(users::id.eq(1))
        .to_sql();
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
    let (sql, params) = insert::<Postgres, _>(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .on_conflict_do_nothing(users::email)
        .to_sql();
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
fn upsert_do_update_reuses_update_row_and_supports_returning() {
    let (sql, params) = insert::<Postgres, _>(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .on_conflict_do_update(
            users::email,
            UsersUpdate {
                display_name: Some(Some("A".into())),
                ..Default::default()
            },
        )
        .expect("display_name is set")
        .returning(users::id)
        .to_sql();
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
    let (sql, params) = delete::<Postgres, _>(users::Table)
        .filter(users::id.eq(1))
        .to_sql();
    assert_eq!(sql, "DELETE FROM \"users\" WHERE (\"users\".\"id\" = $1)");
    assert_eq!(params, vec![Value::I32(1)]);
}
