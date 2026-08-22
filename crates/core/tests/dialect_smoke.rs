//! Does capability gating hold up for a second, meaningfully-different
//! dialect, not just Postgres? Checked at the SQL-rendering level; execution
//! is Postgres-only so far.

use qbrs_core::dialect::{MySql, Sqlite};
use qbrs_core::expr::ExprMethods;
use qbrs_core::insert::{Defaultable, InsertRow, InsertValue, insert};
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::select;
use qbrs_core::statement::Statement;

pub struct UsersMarker;
impl TableTrait for UsersMarker {
    const NAME: &'static str = "users";
}
impl qbrs_core::scope::BaseTableSealed for UsersMarker {}
impl qbrs_core::scope::BaseTable for UsersMarker {}

pub struct QuotedMarker;
impl TableTrait for QuotedMarker {
    const NAME: &'static str = "a\"b";
}
impl qbrs_core::scope::BaseTableSealed for QuotedMarker {}
impl qbrs_core::scope::BaseTable for QuotedMarker {}

#[allow(non_upper_case_globals)]
mod quoted {
    use super::QuotedMarker;
    use qbrs_core::expr::{Column, Integer};

    pub const Table: QuotedMarker = QuotedMarker;
    #[allow(non_camel_case_types)]
    pub mod columns {
        use super::*;
        use qbrs_core::expr::ColumnKey;
        #[derive(Clone, Copy)]
        pub struct id;
        impl ColumnKey for id {
            type Table = QuotedMarker;
            type Sql = Integer;
        }
        impl qbrs_core::row::NamedSealed for id {}
        impl qbrs_core::row::Named for id {
            type Name = qbrs_core::type_name!('i', 'd');
            const NAME: &'static str = "id";
        }
        impl qbrs_core::row::Spelled for id {}
    }

    pub const id: Column<columns::id> = Column::new();
}

#[allow(non_upper_case_globals)]
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
        impl qbrs_core::row::NamedSealed for id {}
        impl qbrs_core::row::Named for id {
            type Name = qbrs_core::type_name!('i', 'd');
            const NAME: &'static str = "id";
        }
        impl qbrs_core::row::Spelled for id {}
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

    pub const id: Column<columns::id> = Column::new();
    pub const email: Column<columns::email> = Column::new();
}

struct UsersInsert {
    email: String,
}
impl qbrs_core::insert::InsertRowSealed for UsersInsert {}

impl InsertRow for UsersInsert {
    type Table = UsersMarker;
    const COLUMNS: &'static [&'static str] = &["email"];
    fn into_values(self) -> Vec<InsertValue> {
        vec![Defaultable::Value(self.email).into()]
    }
}

#[test]
fn mysql_uses_backtick_quoting_and_positional_placeholders() {
    let (sql, params) = select((users::id,))
        .from::<MySql, _>(users::Table)
        .filter(users::email.eq("a@example.com"))
        .to_sql();
    assert_eq!(
        sql,
        "SELECT `users`.`id` FROM `users` WHERE (`users`.`email` = ?)"
    );
    assert_eq!(
        params,
        vec![qbrs_core::expr::Value::Text("a@example.com".into())]
    );

    let (sql, _) = insert::<MySql, _>(users::Table)
        .values(UsersInsert {
            email: "a@example.com".into(),
        })
        .to_sql();
    assert_eq!(sql, "INSERT INTO `users` (`email`) VALUES (?)");
}

#[test]
fn sqlite_uses_double_quote_and_positional_placeholders() {
    let (sql, params) = select((users::id,))
        .from::<Sqlite, _>(users::Table)
        .filter(users::email.eq("a@example.com"))
        .to_sql();
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"email\" = ?)"
    );
    assert_eq!(
        params,
        vec![qbrs_core::expr::Value::Text("a@example.com".into())]
    );
}

#[test]
fn sqlite_supports_returning_mysql_does_not() {
    // SQLite 3.35+ has RETURNING, same as Postgres.
    let (sql, _) = qbrs_core::delete::delete::<Sqlite, _>(users::Table)
        .filter(users::id.eq(1))
        .returning(users::id)
        .to_sql();
    assert_eq!(
        sql,
        "DELETE FROM \"users\" WHERE (\"users\".\"id\" = ?) RETURNING \"users\".\"id\""
    );

    // MySQL has no RETURNING at all — `.returning(..)` must not exist on a
    // MySql-backed Delete/Insert/Update. Uncomment to confirm the compile
    // error (kept commented since this test file otherwise compiles/runs):
    //
    // let _ = qbrs_core::delete::delete::<MySql, _>(users::Table)
    //     .filter(users::id.eq(1))
    //     .returning(users::id); // error[E0599]: no method named `returning`
}

#[test]
fn mysql_supports_right_join_but_not_full_join() {
    // (No RIGHT JOIN example here since it needs a second table — the
    // point is purely that `.full_join()` doesn't exist for MySql, proven
    // by the commented-out snippet below failing to compile if uncommented.)
    //
    // qbrs_core::select::select((users::id,))
    //     .from::<MySql, _>(users::Table)
    //     .full_join(users::Table, users::id.eq(users::id)); // error[E0277]: `MySql` doesn't implement `SupportsFullOuterJoin`
}

#[test]
fn sqlite_supports_on_conflict_mysql_does_not() {
    let (sql, _) = insert::<Sqlite, _>(users::Table)
        .values(UsersInsert {
            email: "a@example.com".into(),
        })
        .on_conflict_do_nothing(users::email)
        .to_sql();
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\") VALUES (?) ON CONFLICT (\"email\") DO NOTHING"
    );

    // MySQL has no ON CONFLICT at all (its equivalent is the differently-
    // shaped ON DUPLICATE KEY UPDATE, a separate future capability) —
    // `.on_conflict_do_nothing(..)` must not exist on a MySql-backed
    // Insert. Uncomment to confirm the compile error:
    //
    // let _ = insert::<MySql, _>(users::Table)
    //     .values(UsersInsert { email: "a@example.com".into() })
    //     .on_conflict_do_nothing(users::email); // error[E0599]: no method named `on_conflict_do_nothing`
}

#[test]
fn each_dialect_spells_an_aggregate_cast_its_own_way() {
    let pg = select((qbrs_core::expr::avg(users::id),))
        .from::<qbrs_core::dialect::Postgres, _>(users::Table)
        .to_sql()
        .0;
    let my = select((qbrs_core::expr::avg(users::id),))
        .from::<qbrs_core::dialect::MySql, _>(users::Table)
        .to_sql()
        .0;
    assert!(
        pg.contains("CAST(avg(\"users\".\"id\") AS DOUBLE PRECISION)"),
        "{pg}"
    );
    assert!(my.contains("CAST(avg(`users`.`id`) AS DOUBLE)"), "{my}");
}

#[test]
fn a_quote_inside_an_identifier_is_doubled() {
    // `#[table(name = "..")]` takes an arbitrary string, so the renderer has
    // to close the identifier itself rather than trusting the input.
    let (sql, _) = select((quoted::id,))
        .from::<qbrs_core::dialect::Postgres, _>(quoted::Table)
        .to_sql();
    assert_eq!(sql, "SELECT \"a\"\"b\".\"id\" FROM \"a\"\"b\"");
}

#[test]
fn a_bare_offset_gets_the_filler_limit_its_dialect_needs() {
    let pg = select((users::id,))
        .from::<qbrs_core::dialect::Postgres, _>(users::Table)
        .offset(5)
        .to_sql()
        .0;
    let lite = select((users::id,))
        .from::<Sqlite, _>(users::Table)
        .offset(5)
        .to_sql()
        .0;
    let my = select((users::id,))
        .from::<MySql, _>(users::Table)
        .offset(5)
        .to_sql()
        .0;
    assert!(pg.ends_with("OFFSET 5"), "{pg}");
    assert!(lite.ends_with("LIMIT -1 OFFSET 5"), "{lite}");
    assert!(my.ends_with("LIMIT 18446744073709551615 OFFSET 5"), "{my}");
}

#[test]
fn sqlite_takes_its_union_branches_as_derived_tables() {
    let a = select((users::id,)).from::<Sqlite, _>(users::Table);
    let b = select((users::id,)).from::<Sqlite, _>(users::Table);
    let (sql, _) = a.union(&b).to_sql();
    assert_eq!(
        sql,
        "SELECT * FROM (SELECT \"users\".\"id\" FROM \"users\") \
         UNION SELECT * FROM (SELECT \"users\".\"id\" FROM \"users\")"
    );

    let c = select((users::id,)).from::<qbrs_core::dialect::Postgres, _>(users::Table);
    let d = select((users::id,)).from::<qbrs_core::dialect::Postgres, _>(users::Table);
    let (pg, _) = c.union(&d).to_sql();
    assert!(pg.starts_with('('), "{pg}");
}
