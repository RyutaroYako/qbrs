//! Common table expressions: `WITH name AS (..) SELECT .. FROM name ..`.
//!
//! A CTE's column names are just positions in a `SELECT` list, with nothing
//! at the type level for `.from(some_cte)` to check against. `with!{}`
//! (`core/src/with_macro.rs`) declares them, generating the same `Table`
//! marker and `Column` consts `#[derive(Table)]` does — so a bound CTE *is*
//! a real table to `Scope`/`Find`/`Superset`/`Selection`, with no parallel
//! virtual-table machinery.
//!
//! Being syntactic, `with!{}` can't see the query it will be paired with.
//! `with()` checks the two agree via the `RowValues::Values = Marker::Shape`
//! bound, the same associated-type equality `select::SetOp` uses for `UNION`
//! branches. The comparison is on the body's *values*, not on its row keys:
//! a CTE declares its own column names, so the body's keys are by definition
//! different ones.
//!
//! **Known limitations**: non-recursive, single-level CTEs only.
//! `WITH RECURSIVE` and a CTE referencing an earlier one in the same
//! `.with().with()` chain both need a CTE to be nameable *inside* another
//! query being built.

use std::marker::PhantomData;

use crate::dialect::Dialect;
use crate::render::Fragment;
use crate::row::RowValues;
use crate::scope::Table;
use crate::select::{Select, Selection};

/// Implemented by a `with!{}`-generated pseudo-table's `Table` marker,
/// pinning down the exact tuple of native types its CTE body must produce.
///
/// `COLUMN_NAMES` is rendered as an explicit column list
/// (`WITH name (col1, col2) AS (..)`) so the outer query sees the declared
/// names whatever the inner query produced — a computed expression like
/// `sql!(BigInt, "sum(orders.total)")` has no usable name of its own.
pub trait CteShape: Table {
    type Shape;
    const COLUMN_NAMES: &'static [&'static str];
}

/// A `WITH name AS (..)` binding, ready to attach to an outer query via
/// `select(..).with(cte).from(name::Table)...`.
pub struct Cte<D, Marker> {
    pub(crate) name: &'static str,
    pub(crate) column_names: &'static [&'static str],
    pub(crate) body: Fragment,
    _marker: PhantomData<fn() -> (D, Marker)>,
}

/// Builds a `Cte` from `query`, checking that `query`'s selected columns
/// match `Marker`'s `with!{}`-declared shape exactly — same count, order,
/// and native types.
pub fn with<D: Dialect, Marker: CteShape, Scope, Sel, Idx>(
    _marker: Marker,
    query: &Select<D, Scope, Sel>,
) -> Cte<D, Marker>
where
    Sel: Selection<Scope, Idx>,
    Sel::Output: RowValues<Values = Marker::Shape>,
{
    Cte {
        name: Marker::NAME,
        column_names: Marker::COLUMN_NAMES,
        body: query.fragment::<Idx>(),
        _marker: PhantomData,
    }
}
