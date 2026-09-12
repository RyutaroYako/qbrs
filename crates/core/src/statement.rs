//! What `INSERT`, `UPDATE` and `DELETE` have in common: they write to one
//! table, and any of them can be asked for the rows it touched.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsReturning};
use crate::expr::Value;
use crate::render::{QuerySink, SelectItem, Sink, render_select_list};
use crate::scope::{Cons, Nil, NotNull, Table, TableSlot};
use crate::select::Selection;

/// A statement that writes to a single table. Sealed: the three writing
/// statements are the whole set, and `Returning` is defined against this
/// rather than against each of them.
pub trait Statement: private::Sealed {
    type Dialect: Dialect;
    type Table: Table;

    /// Renders into whatever sink the statement is going into — a
    /// `QuerySink` when it is the statement, a `FragmentSink` when it is a
    /// CTE body whose placeholders the host query will number.
    #[doc(hidden)]
    fn render_into(&self, sink: &mut dyn Sink);

    /// The dialect is an argument for the reason `Select::to_sql`'s is.
    fn to_sql(&self, _dialect: Self::Dialect) -> (String, Vec<Value>) {
        let mut sink = QuerySink::<Self::Dialect>::new();
        self.render_into(&mut sink);
        sink.finish()
    }

    /// `RETURNING`, on whichever of the three this is — the clause is the
    /// same clause. A distinct type rather than `Self` with a flag set: the
    /// execution layer needs `Sel`'s concrete type to know what to decode a
    /// returned row into, and an optional field would erase it.
    ///
    /// The selection is checked against [`WrittenTable`] — this statement's
    /// own row, which is all SQL's `RETURNING` can name. To hand back a
    /// column of another table alongside it, bind this statement as a CTE
    /// body ([`cte::with`](crate::cte::with)) and join from the outer
    /// query: the write and the read it feeds stay one statement.
    fn returning<Sel, Idx>(self, sel: Sel) -> Returning<Self, Sel>
    where
        Self: Sized,
        Self::Dialect: SupportsReturning,
        Sel: Selection<WrittenTable<Self::Table>, Idx>,
    {
        Returning {
            returning: sel.items(),
            statement: self,
            _marker: PhantomData,
        }
    }
}

pub(crate) mod private {
    pub trait Sealed {}
}

/// The scope a `RETURNING` clause is checked against: the table being
/// written to, and nothing else.
pub type WrittenTable<T> = Cons<TableSlot<T, NotNull>, Nil>;

/// `<statement> RETURNING <selection>`. A distinct type rather than a flag
/// on the statement, because `Sel` has to survive to the point rows are
/// decoded — and one type rather than three, because the clause is the same
/// clause whichever statement it follows.
pub struct Returning<S, Sel> {
    pub(crate) statement: S,
    pub(crate) returning: Vec<SelectItem>,
    pub(crate) _marker: PhantomData<fn() -> Sel>,
}

impl<S: Statement, Sel> Returning<S, Sel> {
    pub fn to_sql(&self, _dialect: S::Dialect) -> (String, Vec<Value>) {
        let mut sink = QuerySink::<S::Dialect>::new();
        self.render_into(&mut sink);
        sink.finish()
    }

    pub(crate) fn render_into(&self, sink: &mut dyn Sink) {
        self.statement.render_into(sink);
        sink.text(" RETURNING ");
        render_select_list::<S::Dialect>(&self.returning, sink);
    }
}
