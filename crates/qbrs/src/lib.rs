//! The crate users depend on: `qbrs-core`'s query builder plus the derive
//! and macros from `qbrs-macros`.

pub use qbrs_core::*;
pub use qbrs_macros::{FromRow, Table, label};

/// Everything a query needs in scope: the builder entry points, the
/// extension traits whose methods would otherwise be unreachable
/// (`.eq()`, `.asc()`, `row.count()`, `.into_tuples()`), the dialect
/// markers, and the macros.
pub mod prelude {
    pub use crate::{FromRow, Table, label};
    pub use qbrs_core::dialect::{MySql, Postgres, Sqlite};
    pub use qbrs_core::expr::{ExprMethods, HasCount, IntoExpr, TextExprMethods, count};
    pub use qbrs_core::row::{IntoStructs, IntoTuples, Row};
    pub use qbrs_core::select::{OrderExt, select};
    pub use qbrs_core::window::{
        HasDenseRank, HasRank, HasRowNumber, dense_rank, rank, row_number, window,
    };
    pub use qbrs_core::{prepare, sql, with};
}
