//! The crate users depend on: `qbrs-core`'s query builder plus the derive
//! and macros from `qbrs-macros`.

pub use qbrs_core::*;
pub use qbrs_macros::{FromRow, Table, label, with};

/// A name belongs here if user source has to spell it: the builder entry
/// points, the extension traits whose methods would otherwise be
/// unreachable, the dialect markers, the macros, and every type that turns
/// up in a signature or a type alias a user may have to write — a `Scope`
/// list, a `Row` list, a `Predicate`, an insert field's `Defaultable`.
/// Nothing that only ever appears as `impl Trait` in an argument position.
pub mod prelude {
    pub use crate::{FromRow, Table, label, with};
    pub use qbrs_core::cte::with as bind_cte;
    pub use qbrs_core::delete::delete;
    pub use qbrs_core::dialect::{MySql, Postgres, Sqlite};
    pub use qbrs_core::expr::Column;
    pub use qbrs_core::expr::{
        AliasExt, BigInt, Bool, Bytes, ExprMethods, HasCount, Integer, Real, SortDir, Text,
        TextExprMethods, avg, count, count_of, max, min, sum,
    };
    pub use qbrs_core::expr::{Declared, Expr, Keyed};
    pub use qbrs_core::insert::{Defaultable, insert};
    pub use qbrs_core::row::{IntoStructs, IntoTuples, Named, Row, RowCons, RowNil};
    pub use qbrs_core::scope::{
        Cons, Find, MaybeNull, Nil, NotNull, Nullable, Superset, TableSlot,
    };
    pub use qbrs_core::select::{
        Correlated, DynSelect, OrderExt, Predicate, Prepared, Select, SetOp, predicate, select,
    };
    pub use qbrs_core::update::update;
    pub use qbrs_core::window::{
        HasDenseRank, HasRank, HasRowNumber, dense_rank, rank, row_number, window,
    };
    pub use qbrs_core::{prepare, sql};
}
