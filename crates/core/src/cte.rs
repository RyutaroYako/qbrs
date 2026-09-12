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
//! round-trip instead of two statements in a transaction. Every part of
//! such a statement sees one snapshot, so the outer query reads the rows
//! the body returned and not the table it wrote.
//!
//! **Known limitations**: non-recursive, single-level CTEs only.
//! `WITH RECURSIVE` and a CTE body referencing another CTE both need a CTE
//! to be nameable *inside* another query being built.
//!
//! A write body must be the whole statement's, and that one is unchecked:
//! Postgres takes a data-modifying `WITH` at the top level only, so a query
//! binding one and then used as an `EXISTS`/`IN` subquery or a set-operation
//! branch is refused by the server (`0A000`) rather than by the compiler.
//! Saying it in the types would mean tracking, on every `Select`, whether
//! its scope was reached through such a binding — which is what `Scope`
//! deliberately does not carry, since a scope is the tables in it and
//! nothing about how they got there.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsDataModifyingCte};
use crate::render::{Fragment, FragmentSink, Sink};
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
    fn render_body(&self, sink: &mut dyn Sink);
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

    fn render_body(&self, sink: &mut dyn Sink) {
        Select::render_body_into::<Idx>(self, sink);
    }
}

impl<D, S, Sel, Idx> cte_body::Sealed<D, Idx> for Returning<S, Sel>
where
    D: SupportsDataModifyingCte,
    S: Statement<Dialect = D>,
    Sel: Selection<WrittenTable<S::Table>, Idx>,
{
}

impl<D, S, Sel, Idx> CteBody<D, Idx> for Returning<S, Sel>
where
    D: SupportsDataModifyingCte,
    S: Statement<Dialect = D>,
    Sel: Selection<WrittenTable<S::Table>, Idx>,
{
    type Output = Sel::Output;

    fn render_body(&self, sink: &mut dyn Sink) {
        Returning::render_into(self, sink);
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
    let mut sink = FragmentSink::new();
    body.render_body(&mut sink);
    Cte {
        body: sink.finish(),
        _marker: PhantomData,
    }
}
