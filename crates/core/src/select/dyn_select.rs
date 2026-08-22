//! `DynSelect`: `Select` with its join-topology-tracking `Scope` erased.

use std::marker::PhantomData;

use super::{Select, SelectBody, Selection};
use crate::dialect::Dialect;
use crate::expr::Value;
use crate::render::{QuerySink, SelectItem};

/// The one unavoidable escape hatch in this design: a single static type
/// cannot mean "this table is joined" in one branch of an `if` and "it
/// isn't" in another. Only the join skeleton is erased — every column
/// reference was already checked against a concrete `Scope` before
/// `.erase()`, and predicates keep the same closed `ExprKind`/`Value`
/// representation used everywhere else, with no `Box<dyn _>` involved.
///
/// Composition happens *before* erasure, which is why `DynSelect` offers no
/// further `.filter()`/`.join()`. It exists only to let two fully-built
/// branches with different join topology unify into one value.
pub struct DynSelect<D, Output> {
    body: SelectBody,
    selection: Vec<SelectItem>,
    _marker: PhantomData<fn() -> (D, Output)>,
}

impl<D, Scope, Sel> Select<D, Scope, Sel> {
    /// Erases `Scope`. `Sel::items()` runs here, while `Scope`/`Idx` are
    /// still known; the resulting `Vec<SelectItem>` and the plain-Rust
    /// `Output` type are all `DynSelect` needs afterwards.
    pub fn erase<Idx>(self) -> DynSelect<D, Sel::Output>
    where
        Sel: Selection<Scope, Idx>,
    {
        DynSelect {
            body: self.body,
            selection: self.selection.items(),
            _marker: PhantomData,
        }
    }
}

impl<D, Output> DynSelect<D, Output> {
    /// `LIMIT`/`OFFSET` survive erasure because they reference nothing: a
    /// row count needs no proof that a table is joined. `order_by` doesn't
    /// follow them here — a sort key is a column reference, and the scope
    /// that would justify it is exactly what `.erase()` gave up.
    pub fn limit(mut self, n: impl super::IntoLimit) -> Self {
        self.body.limit = Some(n.into_limit());
        self
    }

    pub fn offset(mut self, n: impl super::IntoLimit) -> Self {
        self.body.offset = Some(n.into_limit());
        self
    }
}

impl<D: Dialect, Output> DynSelect<D, Output> {
    /// The same total `Select::count_sql` renders. Paging is the reason
    /// `LIMIT`/`OFFSET` survive erasure, and a page needs a total.
    pub fn count_sql(&self) -> (String, Vec<Value>) {
        self.body.count_sql::<D>(&self.selection)
    }

    pub fn to_sql(&self) -> (String, Vec<Value>) {
        let mut sink = QuerySink::<D>::new();
        self.body.render_into::<D>(&self.selection, &mut sink);
        sink.finish()
    }
}
