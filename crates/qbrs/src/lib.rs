//! The crate users depend on: `qbrs-core`'s query builder plus the derive
//! and macros from `qbrs-macros`.

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
    #[cfg(feature = "decimal")]
    pub use qbrs_core::expr::Numeric;
    pub use qbrs_core::expr::{Agg, Avg, Count, CountOf, Max, Min, Sum};
    pub use qbrs_core::expr::{AssignsTo, BoolLike, Comparable, SqlType, TextLike, Writable, null};
    pub use qbrs_core::expr::{
        BigInt, Bool, Bytes, ExprMethods, HasCount, Integer, LabelExt, Real, SortDir, Text, Value,
        all_of, any_of, avg, count, count_of, max, min, sum,
    };
    #[cfg(feature = "chrono")]
    pub use qbrs_core::expr::{Date, Timestamptz};
    pub use qbrs_core::expr::{Declared, Expr, IntoExpr, Keyed, Labeled};
    pub use qbrs_core::insert::{
        Defaultable, Insert, InsertRow, InsertSeed, IntoColumnValue, Missing, NothingToInsert,
        insert,
    };
    pub use qbrs_core::row::{Anon, FromRow, IntoStructs, IntoTuples, Named, Row, RowCons, RowNil};
    pub use qbrs_core::scope::{
        BaseTable, Concat, Cons, Find, Here, MaybeNull, Nil, NotNull, Nullable, Position, Superset,
        TableSlot, There,
    };
    pub use qbrs_core::select::All;
    pub use qbrs_core::select::{
        Condition, DynSelect, Exists, GroupBy, Grouping, JoinSource, OrderExt, OrderKey, Ordinal,
        OrdinalKey, Predicate, Prepared, Select, SelectSeed, Selection, SetOp, SortBy, SortKey,
        Total, grouping, nth, predicate, select, sort_key,
    };
    pub use qbrs_core::statement::{Returning, Statement, WrittenTable};
    pub use qbrs_core::update::{Assignments, NothingToSet, Update, UpdateRow, UpdateSeed, update};
    pub use qbrs_core::window::{
        DenseRank, HasDenseRank, HasRank, HasRowNumber, Rank, RowNumber, Window, WindowFunc,
        dense_rank, rank, row_number, window,
    };
    pub use qbrs_core::{prepare, sql};
}
