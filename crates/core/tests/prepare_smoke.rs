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
    }

    pub const email: Column<columns::email> = Column::new();
}

prepare! {
    struct ByEmail { email: Text }
}

#[test]
fn a_prepared_query_resolved_twice_keeps_its_sql_and_binds_each_params_value() {
    let query = select((users::email,))
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare::<ByEmail, _>(Postgres);

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
fn a_placeholder_that_no_params_field_fills_is_an_error_naming_it() {
    // Hand-built rather than taken from `ByEmail::email()`, which cannot name
    // a field `ByEmail` lacks.
    let bogus = qbrs_core::expr::placeholder::<Text>("not_email");
    let query = select((users::email,))
        .from(users::Table)
        .filter(users::email.eq(bogus))
        .prepare::<ByEmail, _>(Postgres);

    let err = query
        .resolve(ByEmail {
            email: "a@example.com".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.0, "not_email");
}

prepare! {
    struct ByOtherEmail { email: Text }
}

#[test]
fn a_query_resolved_with_another_prepare_structs_same_named_field_is_an_error() {
    let query = select((users::email,))
        .from(users::Table)
        .filter(users::email.eq(ByEmail::email()))
        .prepare::<ByOtherEmail, _>(Postgres);

    let resolved = query.resolve(ByOtherEmail {
        email: "a@example.com".into(),
    });
    assert!(resolved.is_err(), "{resolved:?}");
}
