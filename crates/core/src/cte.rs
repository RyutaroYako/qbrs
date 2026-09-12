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
//! A body can be a write statement with a `RETURNING`, where the dialect
//! has data-modifying CTEs — which is Postgres alone. That is what turns
//! "write a row, then read a value the row doesn't hold" into one
//! round-trip instead of two statements in a transaction.
//!
//! **Known limitations**: non-recursive, single-level CTEs only.
//! `WITH RECURSIVE` and a CTE body referencing another CTE both need a CTE
//! to be nameable *inside* another query being built.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsDataModifyingCte};
use crate::render::{Fragment, FragmentSink};
use crate::row::{Row, SameShape};
use crate::scope::Table;
use crate::select::{Select, Selection};
use crate::statement::{Returning, Statement, WrittenTable};

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

/// What a `WITH` clause can bind: a `SELECT`, or — where the dialect has
/// data-modifying CTEs — an `INSERT`/`UPDATE`/`DELETE` with a `RETURNING`.
/// `Output` is the row the body produces, which is what [`with`] checks
/// against the declared shape.
///
/// Sealed, for the reason [`CteShape`] is: it pairs a type-level claim
/// with the rendered body that is supposed to match it, and an impl saying
/// otherwise would have the outer query read columns by keys the body
/// never selected.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't something a `WITH` clause can bind",
    label = "a `SELECT`, or a write statement with `.returning(..)` on a dialect that has data-modifying CTEs",
    note = "an `INSERT`/`UPDATE`/`DELETE` body is Postgres's alone, and needs the `RETURNING` that gives the CTE its columns"
)]
pub trait CteBody<D, Idx>: cte_body::Sealed<D, Idx> {
    /// The row the body produces — a selection's `Output` either way.
    type Output;

    #[doc(hidden)]
    fn fragment(&self) -> Fragment;
}

mod cte_body {
    /// Carries the trait's own parameters, so there is nothing to project
    /// and nothing an outside crate can implement.
    pub trait Sealed<D, Idx> {}
}

impl<D: Dialect, Scope, Sel: Selection<Scope, Idx>, Idx> cte_body::Sealed<D, Idx>
    for Select<D, Scope, Sel>
{
}

impl<D: Dialect, Scope, Sel: Selection<Scope, Idx>, Idx> CteBody<D, Idx> for Select<D, Scope, Sel> {
    type Output = Sel::Output;

    fn fragment(&self) -> Fragment {
        Select::fragment::<Idx>(self)
    }
}

impl<D, S, Sel, Idx> cte_body::Sealed<D, Idx> for Returning<S, Sel>
where
    D: SupportsDataModifyingCte,
    S: Statement<Dialect = D>,
    Sel: Selection<WrittenTable<S::Table>, Idx>,
{
}

/// A write statement's rows, read by the query it is bound into. Postgres
/// runs it once, whether or not the outer query reads from it, and every
/// part of the statement sees the same snapshot — so a table the CTE writes
/// still reads as it was, and two CTEs writing one row leave an order
/// nothing here decides.
impl<D, S, Sel, Idx> CteBody<D, Idx> for Returning<S, Sel>
where
    D: SupportsDataModifyingCte,
    S: Statement<Dialect = D>,
    Sel: Selection<WrittenTable<S::Table>, Idx>,
{
    type Output = Sel::Output;

    fn fragment(&self) -> Fragment {
        let mut sink = FragmentSink::new();
        Returning::render_into(self, &mut sink);
        sink.finish()
    }
}

/// Builds a `Cte` from `body`, checking that the columns it produces match
/// `Marker`'s `with!{}`-declared shape exactly — same count, order, names,
/// and native types.
pub fn with<D: Dialect, Marker: CteShape, Body, Idx>(_marker: Marker, body: &Body) -> Cte<D, Marker>
where
    Body: CteBody<D, Idx>,
    Body::Output: SameShape<Row<Marker::Row>>,
{
    Cte {
        body: body.fragment(),
        _marker: PhantomData,
    }
}
