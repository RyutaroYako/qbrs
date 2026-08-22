//! What `INSERT`, `UPDATE` and `DELETE` have in common: they write to one
//! table, and any of them can be asked for the rows it touched.

use std::marker::PhantomData;

use crate::dialect::Dialect;
use crate::expr::Value;
use crate::render::{QuerySink, SelectItem, Sink, render_select_list};
use crate::scope::{Cons, Nil, NotNull, Table, TableSlot};

/// A statement that writes to a single table. Sealed: the three writing
/// statements are the whole set, and `Returning` is defined against this
/// rather than against each of them.
pub trait Statement: private::Sealed {
    type Dialect: Dialect;
    type Table: Table;

    #[doc(hidden)]
    fn render(&self) -> QuerySink<Self::Dialect>;

    fn to_sql(&self) -> (String, Vec<Value>) {
        self.render().finish()
    }
}

mod private {
    pub trait Sealed {}
}

#[doc(hidden)]
pub use private::Sealed as StatementSealed;

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
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        let mut sink = self.statement.render();
        sink.text(" RETURNING ");
        render_select_list::<S::Dialect>(&self.returning, &mut sink);
        sink.finish()
    }
}
