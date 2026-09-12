//! A Drizzle-flavored, type-safe SQL query builder for Rust — not an ORM and
//! not a raw-SQL macro. Column and join references are checked at compile
//! time without giving up dynamic composition, and scope resolution stays
//! linear as the join count grows.
//!
//! This is the facade users depend on: [`qbrs_core`]'s builders and renderer
//! plus the derive and macros from `qbrs-macros`. Nothing here touches a
//! database — executing a rendered statement against Postgres is
//! [`qbrs-sqlx`](https://docs.rs/qbrs-sqlx).
//!
//! ```
//! use qbrs::prelude::*;
//!
//! #[derive(Table)]
//! #[table(name = "users")]
//! struct Users {
//!     #[column(primary_key, generated)]
//!     id: i64,
//!     email: String,
//!     #[column(default)]
//!     active: bool,
//! }
//!
//! #[derive(Table)]
//! #[table(name = "orders")]
//! struct Orders {
//!     #[column(primary_key, generated)]
//!     id: i64,
//!     user_id: i64,
//!     total: i64,
//! }
//!
//! fn main() {
//!     let (sql, params) = select((users::id, orders::total))
//!         .from(users::Table)
//!         .left_join(orders::Table, orders::user_id.eq(users::id))
//!         .filter(users::active.eq(true))
//!         .to_sql(Postgres);
//!
//!     assert_eq!(
//!         sql,
//!         "SELECT \"users\".\"id\", \"orders\".\"total\" FROM \"users\" \
//!          LEFT JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") \
//!          WHERE (\"users\".\"active\" = $1)"
//!     );
//!     assert_eq!(params.len(), 1);
//! }
//! ```
//!
//! Dropping the `.left_join(..)` line makes that query a compile error rather
//! than a runtime one: `orders` is in scope for neither the selection nor the
//! `ON` clause. The join is also what decides nullability — `orders::total`
//! decodes as `Option<i64>` above and as `i64` after an `INNER JOIN`.

pub use qbrs_core::*;
pub use qbrs_macros::{FromRow, Table, label, with};

/// A name belongs here if user source has to spell it: the builder entry
/// points, the extension traits whose methods would otherwise be
/// unreachable, the dialect markers, the macros, and every type that turns
/// up in a signature or a type alias a user may have to write — a `Scope`
/// list, a `Row` list, a `Predicate`, an insert field's `Defaultable`, and
/// the three `*Seed`s a single-dialect app wraps to stop repeating
/// `::<Postgres, _>`.
/// Nothing that only ever appears as `impl Trait` in an argument position.
pub mod prelude {
    pub use crate::{FromRow, Table, label, with};
    pub use qbrs_core::cte;
    pub use qbrs_core::delete::{Delete, delete};
    pub use qbrs_core::dialect::{Dialect, MySql, Postgres, Sqlite};
    pub use qbrs_core::expr::Column;
    #[cfg(feature = "json")]
    pub use qbrs_core::expr::Json;
    #[cfg(feature = "decimal")]
    pub use qbrs_core::expr::Numeric;
    #[cfg(feature = "uuid")]
    pub use qbrs_core::expr::UuidArray;
    pub use qbrs_core::expr::{Agg, Avg, Count, CountOf, Max, Min, StringAgg, Sum};
    pub use qbrs_core::expr::{
        AssignsTo, BoolLike, Comparable, Concatenable, Ordered, SqlType, Summable, TextLike,
        Writable, null,
    };
    pub use qbrs_core::expr::{
        BigInt, BigIntArray, Bool, Bytes, ExprMethods, HasCount, Integer, IntegerArray, LabelExt,
        Real, SortDir, Text, TextArray, Value, all_of, any_of, avg, count, count_of, max, min,
        string_agg, sum,
    };
    #[cfg(feature = "chrono")]
    pub use qbrs_core::expr::{Date, Timestamptz};
    pub use qbrs_core::expr::{Declared, Expr, IntoExpr, Keyed, Labeled};
    pub use qbrs_core::insert::{
        ConflictColumns, ConflictTarget, ConflictUpdate, Defaultable, Insert, InsertRow,
        InsertSeed, InsertSelect, Insertable, IntoColumnValue, Missing, NothingToInsert,
        PartialIndex, WrittenColumns, excluded, insert, partial_index,
    };
    pub use qbrs_core::row::{Anon, FromRow, IntoStructs, IntoTuples, Named, Row, RowCons, RowNil};
    pub use qbrs_core::scope::{
        BaseTable, Concat, Cons, Find, Here, MaybeNull, Nil, NotNull, Nullable, Position, Superset,
        TableSlot, There,
    };
    pub use qbrs_core::select::All;
    pub use qbrs_core::select::{
        Condition, DynSelect, Exists, GroupBy, Grouping, InSubquery, JoinSource, OrderExt,
        OrderKey, Predicate, Prepared, Select, SelectSeed, Selection, SetOp, SortBy, SortKey,
        Total, grouping, predicate, select, sort_key,
    };
    pub use qbrs_core::statement::{Returning, Statement, WrittenTable};
    pub use qbrs_core::update::{Assignments, NothingToSet, Update, UpdateRow, UpdateSeed, update};
    pub use qbrs_core::window::{
        DenseRank, HasDenseRank, HasRank, HasRowNumber, Rank, RowNumber, Window, WindowFunc,
        dense_rank, rank, row_number, window,
    };
    pub use qbrs_core::{prepare, sql};
}
