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
/// The declared row is also where the rendered column list
/// (`WITH name (col1, col2) AS (..)`) comes from, so the header the outer
/// query reads by and the shape the body was checked against are one fact,
/// not two that can disagree.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a `with!{{}}` pseudo-table",
    label = "only a `with!{{}}`-declared name can be bound as a CTE",
    note = "a schema table is already a table — it is selected from directly, with no `WITH` clause to bind"
)]
pub trait CteShape: Table + crate::select::SelectableSealed {
    /// The declared columns as a row — the same `RowCons` chain a selection
    /// produces, so a body is checked against it by the one comparison
    /// `UNION` branches already use: same names, same types, same order.
    type Row: crate::row::ColumnNames;
}

/// A `WITH name AS (..)` binding. It goes where a table goes — `.from(..)`,
/// `.inner_join(..)` — and passing it is what both attaches the `WITH`
/// clause and puts the pseudo-table in scope: one act, so a CTE cannot be
/// selected from without being bound, or bound without being used.
pub struct Cte<D, Marker> {
    body: Fragment,
    _marker: PhantomData<fn() -> (D, Marker)>,
}

impl<D, Marker> Cte<D, Marker> {
    /// The rendered body. What it is bound *as* comes from `Marker`, so a
    /// `Cte` whose name disagrees with its marker is unrepresentable.
    pub(crate) fn into_body(self) -> Fragment {
        self.body
    }
}

impl<D, Marker> Clone for Cte<D, Marker> {
    fn clone(&self) -> Self {
        Cte {
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
        body: query.fragment::<Idx>(),
        _marker: PhantomData,
    }
}
