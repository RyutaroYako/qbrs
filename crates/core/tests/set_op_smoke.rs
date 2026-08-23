//! `UNION`/`UNION ALL`/`INTERSECT`/`EXCEPT` between two `Select`s with
//! different `Scope`s (different source tables) but the same output shape.

use qbrs_core::dialect::Postgres;
use qbrs_core::expr::ExprMethods;
use qbrs_core::scope::Table as TableTrait;
use qbrs_core::select::select;

pub struct UsersMarker;
impl TableTrait for UsersMarker {
    const NAME: &'static str = "users";
}
impl qbrs_core::scope::BaseTableSealed for UsersMarker {}
impl qbrs_core::scope::BaseTable for UsersMarker {}

pub struct ArchivedUsersMarker;
impl TableTrait for ArchivedUsersMarker {
    const NAME: &'static str = "archived_users";
}
impl qbrs_core::scope::BaseTableSealed for ArchivedUsersMarker {}
impl qbrs_core::scope::BaseTable for ArchivedUsersMarker {}

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

#[allow(non_upper_case_globals)]
mod archived_users {
    use super::ArchivedUsersMarker;
    use qbrs_core::expr::{Column, Integer, Text};

    pub const Table: ArchivedUsersMarker = ArchivedUsersMarker;
    #[allow(non_camel_case_types)]
    pub mod columns {
        use super::*;
        use qbrs_core::expr::ColumnKey;
        #[derive(Clone, Copy)]
        pub struct id;
        impl qbrs_core::expr::Writable for id {}
        impl ColumnKey for id {
            type Table = ArchivedUsersMarker;
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
        impl qbrs_core::expr::Writable for email {}
        impl ColumnKey for email {
            type Table = ArchivedUsersMarker;
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

#[test]
fn union_combines_two_different_scopes_and_renumbers_params() {
    // Each branch has its own bound parameter — proves placeholders get
    // renumbered across the splice, not just copied verbatim (which would
    // collide on `$1` twice).
    let live = select((users::id, users::email))
        .from::<Postgres, _>(users::Table)
        .filter(users::id.gt(10));
    let archived = select((archived_users::id, archived_users::email))
        .from::<Postgres, _>(archived_users::Table)
        .filter(archived_users::id.gt(20));

    let (sql, params) = live.union(&archived).to_sql();
    assert_eq!(
        sql,
        "(SELECT \"users\".\"id\", \"users\".\"email\" FROM \"users\" WHERE (\"users\".\"id\" > $1)) \
         UNION \
         (SELECT \"archived_users\".\"id\", \"archived_users\".\"email\" FROM \"archived_users\" WHERE (\"archived_users\".\"id\" > $2))"
    );
    assert_eq!(
        params,
        vec![
            qbrs_core::expr::Value::I32(10),
            qbrs_core::expr::Value::I32(20)
        ]
    );
}

#[test]
fn counting_a_set_op_drops_its_own_paging_but_not_its_branches() {
    let live = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .limit(5);
    let archived = select((archived_users::id,)).from::<Postgres, _>(archived_users::Table);

    let (sql, _params) = live.union(&archived).limit(10).offset(20).count_sql();
    assert_eq!(
        sql,
        "SELECT count(*) FROM ((SELECT \"users\".\"id\" FROM \"users\" LIMIT 5) \
         UNION \
         (SELECT \"archived_users\".\"id\" FROM \"archived_users\")) AS \"qbrs_total\""
    );
}

/// A table wider than the positional view's 16 fields still unions: two
/// selections agree or don't, however wide they are.
#[test]
fn a_wide_selection_can_still_be_a_branch() {
    let left = select((
        users::id,
        users::email,
        users::id,
        users::email,
        users::id,
        users::email,
        users::id,
        users::email,
        users::id,
        users::email,
        users::id,
        users::email,
        users::id,
        users::email,
        users::id,
        users::email,
    ))
    .from::<Postgres, _>(users::Table);
    let right = left.clone();

    let (sql, _params) = left.union(&right).to_sql();
    assert!(sql.contains(" UNION "), "{sql}");
}

#[test]
fn one_column_branches_match_on_their_value_type_alone() {
    let live = select(users::email).from::<Postgres, _>(users::Table);
    let archived = select(archived_users::email).from::<Postgres, _>(archived_users::Table);

    let (sql, _params) = live.union(&archived).to_sql();
    assert_eq!(
        sql,
        "(SELECT \"users\".\"email\" FROM \"users\") \
         UNION \
         (SELECT \"archived_users\".\"email\" FROM \"archived_users\")"
    );
}

#[test]
fn union_all_intersect_except_use_their_own_keywords() {
    let a = select((users::id,)).from::<Postgres, _>(users::Table);
    let b = select((archived_users::id,)).from::<Postgres, _>(archived_users::Table);

    assert_eq!(
        a.union_all(&b).to_sql().0,
        "(SELECT \"users\".\"id\" FROM \"users\") UNION ALL (SELECT \"archived_users\".\"id\" FROM \"archived_users\")"
    );
    assert_eq!(
        a.intersect(&b).to_sql().0,
        "(SELECT \"users\".\"id\" FROM \"users\") INTERSECT (SELECT \"archived_users\".\"id\" FROM \"archived_users\")"
    );
    assert_eq!(
        a.except(&b).to_sql().0,
        "(SELECT \"users\".\"id\" FROM \"users\") EXCEPT (SELECT \"archived_users\".\"id\" FROM \"archived_users\")"
    );
}

#[test]
fn chained_set_ops_and_ordinal_order_by_limit_offset() {
    let a = select((users::id,)).from::<Postgres, _>(users::Table);
    let b = select((archived_users::id,)).from::<Postgres, _>(archived_users::Table);
    let c = select((users::id,))
        .from::<Postgres, _>(users::Table)
        .filter(users::id.eq(1));

    let (sql, params) = a
        .union(&b)
        .union_all(&c)
        .order_by(qbrs_core::select::nth(1).desc())
        .limit(5)
        .offset(2)
        .to_sql();
    assert_eq!(
        sql,
        "(SELECT \"users\".\"id\" FROM \"users\") \
         UNION (SELECT \"archived_users\".\"id\" FROM \"archived_users\") \
         UNION ALL (SELECT \"users\".\"id\" FROM \"users\" WHERE (\"users\".\"id\" = $1)) \
         ORDER BY 1 DESC LIMIT 5 OFFSET 2"
    );
    assert_eq!(params, vec![qbrs_core::expr::Value::I32(1)]);
}

// Uncomment to eyeball the compile error for mismatched output shapes
// (confirmed working — kept out of the normal test run since it's meant to
// fail): `users::email` (Text) vs. `archived_users::id` (Integer) — the two
// branches' `Selection::Output` types differ, so this is a compile error,
// not a runtime "column count/type mismatch" surprise.
//
// #[test]
// fn mismatched_output_shape_is_a_compile_error() {
//     let a = select((users::email,)).from::<Postgres, _>(users::Table);
//     let b = select((archived_users::id,)).from::<Postgres, _>(archived_users::Table);
//     let _ = a.union(&b); // error[E0271]: type mismatch resolving `<... as Selection<...>>::Output == (String,)`
// }
