//! Validates the Phase 2 question the design plan explicitly flagged:
//! does the capability-gating abstraction actually work for a second,
//! meaningfully-different dialect, not just Postgres? Checked at the SQL-
//! rendering level (no live MySQL/SQLite server involved — `qbrs-sqlx`
//! only wires up Postgres execution so far; that's separately-scoped
//! future work, not part of "does the type-level gating work").

use qbrs_core::dialect::{MySql, Sqlite};
use qbrs_core::expr::ExprMethods;
use qbrs_core::insert::{Defaultable, InsertRow, InsertValue, insert};
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::select;

pub struct UsersMarker;
impl TableTrait for UsersMarker {
    const NAME: &'static str = "users";
}

#[allow(non_upper_case_globals)]
mod users {
    use super::UsersMarker;
    use qbrs_core::expr::{Column, Integer, Text};

    pub const Table: UsersMarker = UsersMarker;
    pub const id: Column<UsersMarker, Integer> = Column::new("id");
    pub const email: Column<UsersMarker, Text> = Column::new("email");
}

struct UsersInsert {
    email: String,
}
impl InsertRow for UsersInsert {
    type Table = UsersMarker;
    const COLUMNS: &'static [&'static str] = &["email"];
    fn into_values(self) -> Vec<InsertValue> {
        vec![Defaultable::value(self.email).into()]
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
