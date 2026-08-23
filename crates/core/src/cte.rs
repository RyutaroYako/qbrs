//! Common table expressions: `WITH name AS (..) SELECT .. FROM name ..`.
//!
//! A CTE's column names are just positions in a `SELECT` list, with nothing
//! at the type level for `.from(some_cte)` to check against. `with!{}`
//! declares them, generating everything `#[derive(Table)]` does — so a bound
//! CTE *is* a real table to `Scope`/`Find`/`Superset`/`Selection`, with no
//! parallel virtual-table machinery.
//!
//! Being syntactic, `with!{}` can't see the query it will be paired with.
//! `with()` checks that the body produces the declared columns — the same
//! types *and* the same names, in the same order — through `row::SameShape`,
//! the one comparison a `UNION` branch also goes through. Checking only
//! types would accept a body whose columns are type-compatible but
//! transposed, and the outer query reads those columns by key.
//!
//! **Known limitations**: non-recursive, single-level CTEs only.
//! `WITH RECURSIVE` and a CTE body referencing another CTE both need a CTE
//! to be nameable *inside* another query being built.

use std::marker::PhantomData;

use crate::dialect::Dialect;
use crate::render::Fragment;
use crate::row::{Row, SameShape};
use crate::scope::Table;
use crate::select::{Select, Selection};

/// Implemented by a `with!{}`-generated pseudo-table's `Table` marker,
/// pinning down the exact tuple of native types its CTE body must produce.
///
/// `COLUMN_NAMES` is rendered as an explicit column list
/// (`WITH name (col1, col2) AS (..)`), so the outer query refers to the
/// declared names rather than to whatever Postgres would have called the
/// body's columns.
pub trait CteShape: Table {
    /// The declared columns as a row — the same `RowCons` chain a selection
    /// produces, so a body is checked against it by the one comparison
    /// `UNION` branches already use: same names, same types, same order.
    type Row;
    const COLUMN_NAMES: &'static [&'static str];
}

/// A `WITH name AS (..)` binding. It goes where a table goes — `.from(..)`,
/// `.inner_join(..)` — and passing it is what both attaches the `WITH`
/// clause and puts the pseudo-table in scope: one act, so a CTE cannot be
/// selected from without being bound, or bound without being used.
pub struct Cte<D, Marker> {
    pub(crate) name: &'static str,
    pub(crate) column_names: &'static [&'static str],
    pub(crate) body: Fragment,
    _marker: PhantomData<fn() -> (D, Marker)>,
}

impl<D, Marker> Clone for Cte<D, Marker> {
    fn clone(&self) -> Self {
        Cte {
            name: self.name,
            column_names: self.column_names,
            body: self.body.clone(),
            _marker: PhantomData,
        }
    }
}

/// Builds a `Cte` from `query`, checking that `query`'s selected columns
/// match `Marker`'s `with!{}`-declared shape exactly — same count, order,
/// names, and native types.
pub fn with<D: Dialect, Marker: CteShape, Scope, Sel, Idx>(
    _marker: Marker,
    query: &Select<D, Scope, Sel>,
) -> Cte<D, Marker>
where
    Sel: Selection<Scope, Idx>,
    Sel::Output: SameShape<Row<Marker::Row>>,
{
    Cte {
        name: Marker::NAME,
        column_names: Marker::COLUMN_NAMES,
        body: query.fragment::<Idx>(),
        _marker: PhantomData,
    }
}
