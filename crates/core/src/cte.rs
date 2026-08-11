//! Common table expressions: `WITH name AS (..) SELECT .. FROM name ..`.
//!
//! The hard part the design plan flagged here isn't rendering the `WITH`
//! clause itself (that's a small, mechanical addition to
//! `select::render_select_body`) — it's that a CTE needs to become usable
//! as a real "table" in an outer query's `.from()`/`.join()` chain, with
//! named, typed column accessors, exactly like a real schema table gets
//! from `#[derive(Table)]`. A CTE's column *names* don't exist anywhere at
//! the type level on their own (they're just positions in a `SELECT` list),
//! so there is nothing for `.from(some_cte)` to type-check against unless
//! something declares them.
//!
//! `with!{}` (a declarative macro, `core/src/with_macro.rs`) is that
//! "something": it generates a `scope::Table` marker plus one
//! `Column<Marker, S>` const per declared field, structurally identical to
//! what `#[derive(Table)]` generates for a real table — so once bound, a CTE
//! *is* a real table as far as `Scope`/`Find`/`Superset`/`Selection` are
//! concerned, no separate "virtual table" machinery needed anywhere else in
//! the crate.
//!
//! What `with!{}` *can't* do (being a purely syntactic declarative macro,
//! with no visibility into the actual query it'll eventually be paired
//! with) is verify that its declared column list actually matches the CTE
//! body's real `SELECT` list. That check happens here, at `with()`, via
//! `Selection::Output = Marker::Shape` — an ordinary associated-type-equality
//! bound, the same mechanism `select::SetOp` uses to check two `UNION`
//! branches share an output shape. A `with!{}` declaration whose column
//! types/count/order don't match the query passed to `with()` is a compile
//! error, not a "column 3 doesn't exist" surprise the first time the CTE is
//! queried against a real database.
//!
//! **Known limitations**: only non-recursive, single-level CTEs are
//! supported — `WITH RECURSIVE` (a later CTE definition self-referencing
//! its own name) and a later CTE referencing an earlier one bound in the
//! same `.with().with()` chain both need the CTE to be nameable *inside*
//! another query being built, which is real design work of its own,
//! deferred rather than half-supported.

use std::marker::PhantomData;

use crate::dialect::{Dialect, RawEmbed};
use crate::expr::Value;
use crate::scope::Table;
use crate::select::{Select, Selection};

/// Implemented by a `with!{}`-generated pseudo-table's `Table` marker,
/// pinning down the exact tuple of native types its CTE body must produce.
/// See this module's doc comment for why this check exists and what
/// mechanism enforces it.
///
/// `COLUMN_NAMES` is rendered as the CTE's explicit column-name list
/// (`WITH name (col1, col2) AS (..)`) rather than relying on the inner
/// query's own column names/aliases — necessary because a computed
/// expression (`sql!(BigInt, "sum(orders.total)")`, or any window
/// function) has no real column name of its own for Postgres to expose to
/// the outer query, only whatever the inner `SELECT` naturally produces
/// (often nothing usable, e.g. a bare `sum` for an unaliased aggregate).
/// The explicit list sidesteps that entirely: the outer query only ever
/// sees the `with!{}`-declared names, regardless of what the inner query's
/// own columns happen to be named.
pub trait CteShape: Table {
    type Shape;
    const COLUMN_NAMES: &'static [&'static str];
}

/// A `WITH name AS (..)` binding, ready to attach to an outer query via
/// `select(..).with(cte).from(name::Table)...`. Once bound, `name::Table`
/// (the same zero-sized value `with!{}` generated) behaves exactly like a
/// real table anywhere else in the query — there is no `CteSelect` or
/// similar parallel type, because there is nothing left that needs one.
pub struct Cte<D, Marker> {
    pub(crate) name: &'static str,
    pub(crate) column_names: &'static [&'static str],
    pub(crate) sql: String,
    pub(crate) params: Vec<Value>,
    _marker: PhantomData<fn() -> (D, Marker)>,
}

/// Builds a `Cte` from `query`, checking at compile time that `query`'s
/// selected columns match `Marker`'s `with!{}`-declared shape exactly (same
/// count, same order, same native types) via the `Output = Marker::Shape`
/// bound — see this module's doc comment.
pub fn with<D: Dialect, Marker: CteShape, Scope, Sel, Idx>(
    _marker: Marker,
    query: &Select<D, Scope, Sel>,
) -> Cte<D, Marker>
where
    Sel: Selection<Scope, Idx, Output = Marker::Shape>,
{
    let (sql, params) = query.render_as::<RawEmbed<D>, Idx>();
    Cte {
        name: Marker::NAME,
        column_names: Marker::COLUMN_NAMES,
        sql,
        params,
        _marker: PhantomData,
    }
}
