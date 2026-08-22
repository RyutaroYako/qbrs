//! `prepare!{}`: named, typed placeholders resolved at `.execute()` time
//! rather than baked in at query-build time.

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::{ExprMethods, Text};
use qbrs_core::prepare;
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::select;

pub struct UsersMarker;
impl TableTrait for UsersMarker {
    const NAME: &'static str = "users";
}
impl qbrs_core::scope::BaseTableSealed for UsersMarker {}
impl qbrs_core::scope::BaseTable for UsersMarker {}

#[allow(non_upper_case_globals)]
mod users {
    use super::UsersMarker;
    use qbrs_core::expr::{Column, Text};
    pub const Table: UsersMarker = UsersMarker;
    #[allow(non_camel_case_types)]
    pub mod columns {
        use super::*;
        use qbrs_core::expr::ColumnKey;
        #[derive(Clone, Copy)]
        pub struct email;
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
    }

    pub const email: Column<columns::email> = Column::new();
}

prepare! {
    struct ByEmail { email: Text }
}

#[test]
fn prepared_query_resolves_named_placeholder() {
    let query = select((users::email,))
        .from::<Postgres, _>(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare::<ByEmail, _>();

    let (sql, params) = query
        .resolve(ByEmail {
            email: "a@example.com".to_string(),
        })
        .expect("resolve");
    assert_eq!(
        sql,
        "SELECT \"users\".\"email\" FROM \"users\" WHERE (\"users\".\"email\" = $1)"
    );
    assert_eq!(
        params,
        vec![qbrs_core::expr::Value::Text("a@example.com".to_string())]
    );

    // Same `Prepared` value, reused with a different `Params` — the SQL
    // text (and its placeholder position) doesn't change, only the bound
    // value does.
    let (sql2, params2) = query
        .resolve(ByEmail {
            email: "b@example.com".to_string(),
        })
        .expect("resolve again");
    assert_eq!(sql2, sql);
    assert_eq!(
        params2,
        vec![qbrs_core::expr::Value::Text("b@example.com".to_string())]
    );
}

#[test]
fn missing_placeholder_is_a_typed_error_not_a_panic() {
    // Hand-built with a name that doesn't match `ByEmail`'s field, bypassing
    // the macro's by-construction guarantee on purpose, to prove the
    // mismatch is a recoverable `Result::Err`, not a panic.
    let bogus = qbrs_core::expr::placeholder::<Text>("not_email");
    let query = select((users::email,))
        .from::<Postgres, _>(users::Table)
        .filter(users::email.eq(bogus))
        .prepare::<ByEmail, _>();

    let err = query
        .resolve(ByEmail {
            email: "a@example.com".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.0, "not_email");
}
