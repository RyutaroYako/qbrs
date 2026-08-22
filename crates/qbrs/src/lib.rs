//! The crate users depend on: `qbrs-core`'s query builder plus the derive
//! and macros from `qbrs-macros`.

pub use qbrs_core::*;
pub use qbrs_macros::{FromRow, Table, label, with};

/// Everything a query needs in scope: the builder entry points, the
/// extension traits whose methods would otherwise be unreachable
/// (`.eq()`, `.asc()`, `row.count()`, `.into_tuples()`), the dialect
/// markers, and the macros.
pub mod prelude {
    pub use crate::{FromRow, Table, label, with};
    pub use qbrs_core::delete::delete;
    pub use qbrs_core::dialect::{MySql, Postgres, Sqlite};
    pub use qbrs_core::expr::Column;
    pub use qbrs_core::expr::{
        AliasExt, BigInt, Bool, Bytes, ExprMethods, HasCount, Integer, IntoExpr, Real, SortDir,
        Text, TextExprMethods, avg, count, count_of, max, min, sum,
    };
    pub use qbrs_core::insert::insert;
    pub use qbrs_core::row::{IntoLimit, IntoStructs, IntoTuples, Row, RowCons, RowNil};
    pub use qbrs_core::scope::{Cons, Find, Nil, NotNull, Nullable, TableSlot};
    pub use qbrs_core::select::{
        Correlated, DynSelect, OrderExt, Predicate, Select, predicate, select,
    };
    pub use qbrs_core::update::update;
    pub use qbrs_core::window::{
        HasDenseRank, HasRank, HasRowNumber, dense_rank, rank, row_number, window,
    };
    pub use qbrs_core::{prepare, sql};
}
