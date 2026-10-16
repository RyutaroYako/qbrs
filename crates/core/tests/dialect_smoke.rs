use qbrs_core::dialect::{MySql, Postgres, Sqlite};
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
        impl qbrs_core::expr::WritableSealed for id {}
        impl qbrs_core::expr::Writable for id {}
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
    type Values =
        qbrs_core::row::RowCons<users::columns::email, InsertValue, qbrs_core::row::RowNil>;
    fn into_values(self) -> Self::Values {
        qbrs_core::row::RowCons::new(
            Defaultable::Value(self.email).into(),
            qbrs_core::row::RowNil,
        )
    }
}

#[test]
fn mysql_uses_backtick_quoting_and_positional_placeholders() {
    let (sql, params) = select((users::id,))
        .from(users::Table)
        .filter(users::email.eq("a@example.com"))
        .to_sql(MySql);
    assert_eq!(
        sql,
        "SELECT `users`.`id` FROM `users` WHERE (`users`.`email` = ?)"
    );
    assert_eq!(
        params,
        vec![qbrs_core::expr::Value::Text("a@example.com".into())]
    );

    let (sql, _) = insert(users::Table)
        .values(UsersInsert {
            email: "a@example.com".into(),
        })
        .to_sql(MySql);
    assert_eq!(sql, "INSERT INTO `users` (`email`) VALUES (?)");
}

#[test]
fn sqlite_uses_double_quote_and_positional_placeholders() {
    let (sql, params) = select((users::id,))
        .from(users::Table)
        .filter(users::email.eq("a@example.com"))
        .to_sql(Sqlite);
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
fn a_repeated_value_still_binds_once_per_position_where_placeholders_are_positional() {
    let (sql, params) = select((users::id,))
        .from(users::Table)
        .filter(users::email.eq("a@example.com"))
        .filter(users::email.eq("a@example.com"))
        .to_sql(MySql);
    assert_eq!(
        sql,
        "SELECT `users`.`id` FROM `users` WHERE (`users`.`email` = ?) AND (`users`.`email` = ?)"
    );
    assert_eq!(params.len(), 2);

    let (sql, params) = select((users::id,))
        .from(users::Table)
        .filter(users::email.eq("a@example.com"))
        .filter(users::email.eq("a@example.com"))
        .to_sql(Sqlite);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"email\" = ?) AND (\"users\".\"email\" = ?)"
    );
    assert_eq!(params.len(), 2);
}

#[test]
fn a_sqlite_delete_renders_returning() {
    let (sql, _) = qbrs_core::delete::delete(users::Table)
        .filter(users::id.eq(1))
        .returning(users::id)
        .to_sql(Sqlite);
    assert_eq!(
        sql,
        "DELETE FROM \"users\" WHERE (\"users\".\"id\" = ?) RETURNING \"users\".\"id\""
    );
}

#[test]
fn mysql_renders_a_right_join() {
    let (sql, _) = select((users::id,))
        .from::<MySql, _>(users::Table)
        .right_join(quoted::Table, quoted::id.eq(users::id))
        .to_sql(MySql);
    assert_eq!(
        sql,
        "SELECT `users`.`id` FROM `users` RIGHT JOIN `a\"b` ON (`a\"b`.`id` = `users`.`id`)"
    );
}

#[test]
fn sqlite_renders_on_conflict_do_nothing() {
    let (sql, _) = insert(users::Table)
        .values(UsersInsert {
            email: "a@example.com".into(),
        })
        .on_conflict_do_nothing(users::email)
        .to_sql(Sqlite);
    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\") VALUES (?) ON CONFLICT (\"email\") DO NOTHING"
    );
}

#[test]
fn each_dialect_spells_an_aggregate_cast_its_own_way() {
    let pg = select((qbrs_core::expr::avg(users::id),))
        .from(users::Table)
        .to_sql(Postgres)
        .0;
    let my = select((qbrs_core::expr::avg(users::id),))
        .from(users::Table)
        .to_sql(MySql)
        .0;
    assert!(
        pg.contains("CAST(avg(\"users\".\"id\") AS DOUBLE PRECISION)"),
        "{pg}"
    );
    assert!(my.contains("CAST(avg(`users`.`id`) AS DOUBLE)"), "{my}");
}

#[test]
fn a_double_quote_inside_an_identifier_is_doubled() {
    let (sql, _) = select((quoted::id,)).from(quoted::Table).to_sql(Postgres);
    assert_eq!(sql, "SELECT \"a\"\"b\".\"id\" FROM \"a\"\"b\"");
}

#[test]
fn a_bare_offset_gets_the_filler_limit_its_dialect_needs() {
    let pg = select((users::id,))
        .from(users::Table)
        .offset(5)
        .to_sql(Postgres)
        .0;
    let lite = select((users::id,))
        .from(users::Table)
        .offset(5)
        .to_sql(Sqlite)
        .0;
    let my = select((users::id,))
        .from(users::Table)
        .offset(5)
        .to_sql(MySql)
        .0;
    assert!(pg.ends_with("OFFSET 5"), "{pg}");
    assert!(lite.ends_with("LIMIT -1 OFFSET 5"), "{lite}");
    assert!(my.ends_with("LIMIT 18446744073709551615 OFFSET 5"), "{my}");
}

#[test]
fn sqlite_takes_its_union_branches_as_derived_tables_where_postgres_parenthesises_them() {
    let a = select((users::id,)).from(users::Table);
    let b = select((users::id,)).from(users::Table);
    let (sql, _) = a.union(&b).to_sql(Sqlite);
    assert_eq!(
        sql,
        "SELECT * FROM (SELECT \"users\".\"id\" FROM \"users\") \
         UNION SELECT * FROM (SELECT \"users\".\"id\" FROM \"users\")"
    );

    let c = select((users::id,)).from(users::Table);
    let d = select((users::id,)).from(users::Table);
    let (pg, _) = c.union(&d).to_sql(Postgres);
    assert!(pg.starts_with('('), "{pg}");
}
