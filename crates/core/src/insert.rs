//! `INSERT INTO .. VALUES ..`.
//!
//! A column with a schema default gets a `Defaultable<T>` field, so "omit"
//! and "explicit value" stay distinguishable — and `Defaultable<Option<T>>`
//! when it's also nullable, making that three distinct states. Omission
//! renders as the `DEFAULT` keyword in that row's `VALUES (..)` tuple rather
//! than changing the column list, so rows that omit different fields still
//! share one statement.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsOnConflict, SupportsReturning};
use crate::expr::{Column, ColumnKey, Value};
use crate::render::{SelectItem, render_ident, render_select_list};
use crate::scope::{BaseTable, Cons, Nil, NotNull, Table, TableSlot};
use crate::select::Selection;
use crate::update::UpdateRow;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Defaultable<T> {
    #[default]
    Default,
    Value(T),
}

impl<T> Defaultable<T> {
    pub fn value(v: T) -> Self {
        Defaultable::Value(v)
    }
}

#[derive(Debug, Clone)]
pub enum InsertValue {
    Value(Value),
    /// Renders as the bare `DEFAULT` keyword in the `VALUES` list.
    Default,
}

impl<T: Into<Value>> From<Defaultable<T>> for InsertValue {
    fn from(d: Defaultable<T>) -> Self {
        match d {
            Defaultable::Default => InsertValue::Default,
            Defaultable::Value(v) => InsertValue::Value(v.into()),
        }
    }
}

/// Implemented by the `#[derive(Table)]`-generated `*Insert` struct for each
/// table, which is what pairs `COLUMNS` with a matching `into_values()`
/// order and length; a hand-written impl has to keep the two in step
/// itself.
pub trait InsertRow {
    type Table: Table;
    const COLUMNS: &'static [&'static str];
    fn into_values(self) -> Vec<InsertValue>;
}

/// An `ON CONFLICT` target: one or more columns proven by `T` to belong to
/// the table being inserted into, where a raw `&[&str]` would let a typo
/// through to the database. Implemented for a bare `Column<C>` and for
/// tuples of up to three; add arities as real schemas need them.
pub trait ConflictTarget<T: Table> {
    fn column_names(&self) -> Vec<&'static str>;
}

impl<C: ColumnKey> ConflictTarget<C::Table> for Column<C> {
    fn column_names(&self) -> Vec<&'static str> {
        vec![C::NAME]
    }
}

impl<T: Table, A: ConflictTarget<T>> ConflictTarget<T> for (A,) {
    fn column_names(&self) -> Vec<&'static str> {
        self.0.column_names()
    }
}

impl<T: Table, A: ConflictTarget<T>, B: ConflictTarget<T>> ConflictTarget<T> for (A, B) {
    fn column_names(&self) -> Vec<&'static str> {
        let mut v = self.0.column_names();
        v.extend(self.1.column_names());
        v
    }
}

impl<T: Table, A: ConflictTarget<T>, B: ConflictTarget<T>, C: ConflictTarget<T>> ConflictTarget<T>
    for (A, B, C)
{
    fn column_names(&self) -> Vec<&'static str> {
        let mut v = self.0.column_names();
        v.extend(self.1.column_names());
        v.extend(self.2.column_names());
        v
    }
}

enum ConflictAction {
    DoNothing,
    /// Reuses `UpdateRow`, so `.on_conflict_do_update(..)` takes the same
    /// `*Update` value `.set(..)` does.
    ///
    /// **Known limitation**: only literal/bound values, not
    /// `EXCLUDED.column` (`SET total = users.total + EXCLUDED.total`), which
    /// needs its own typed API.
    DoUpdate(Vec<(&'static str, Value)>),
}

struct ConflictClause {
    target: Vec<&'static str>,
    action: ConflictAction,
}

fn render_conflict_clause<D: Dialect>(
    clause: &ConflictClause,
    sql: &mut String,
    params: &mut Vec<Value>,
) {
    sql.push_str(" ON CONFLICT (");
    for (i, c) in clause.target.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        render_ident::<D>(sql, c);
    }
    sql.push(')');
    match &clause.action {
        ConflictAction::DoNothing => sql.push_str(" DO NOTHING"),
        ConflictAction::DoUpdate(sets) => {
            // Same reason `update` refuses one: `DO UPDATE SET` with nothing
            // after it is not a statement, and `*Update`'s derived `Default`
            // is exactly that shape.
            assert!(
                !sets.is_empty(),
                "ON CONFLICT DO UPDATE must set at least one column; every field of this `*Update` is untouched"
            );
            sql.push_str(" DO UPDATE SET ");
            for (i, (col, val)) in sets.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                render_ident::<D>(sql, col);
                sql.push_str(" = ");
                params.push(val.clone());
                sql.push_str(&D::placeholder(params.len()));
            }
        }
    }
}

pub struct InsertSeed<D, T> {
    _marker: PhantomData<fn() -> (D, T)>,
}

pub fn insert<D, T: BaseTable>(_table: T) -> InsertSeed<D, T> {
    InsertSeed {
        _marker: PhantomData,
    }
}

impl<D, T: Table> InsertSeed<D, T> {
    pub fn values<R: InsertRow<Table = T>>(self, row: R) -> Insert<D, T, R> {
        Insert {
            rows: vec![row.into_values()],
            on_conflict: None,
            _marker: PhantomData,
        }
    }
}

fn render_values_clause<D: Dialect, T: Table, R: InsertRow<Table = T>>(
    rows: &[Vec<InsertValue>],
    on_conflict: &Option<ConflictClause>,
) -> (String, Vec<Value>) {
    let mut sql = String::from("INSERT INTO ");
    render_ident::<D>(&mut sql, T::NAME);
    sql.push_str(" (");
    for (i, c) in R::COLUMNS.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        render_ident::<D>(&mut sql, c);
    }
    sql.push_str(") VALUES ");

    let mut params = Vec::new();
    for (row_i, row) in rows.iter().enumerate() {
        if row_i > 0 {
            sql.push_str(", ");
        }
        sql.push('(');
        for (i, v) in row.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            match v {
                InsertValue::Default => sql.push_str("DEFAULT"),
                InsertValue::Value(v) => {
                    params.push(v.clone());
                    sql.push_str(&D::placeholder(params.len()));
                }
            }
        }
        sql.push(')');
    }

    if let Some(clause) = on_conflict {
        render_conflict_clause::<D>(clause, &mut sql, &mut params);
    }

    (sql, params)
}

pub struct Insert<D, T: Table, R: InsertRow<Table = T>> {
    rows: Vec<Vec<InsertValue>>,
    on_conflict: Option<ConflictClause>,
    _marker: PhantomData<fn() -> (D, T, R)>,
}

impl<D, T: Table, R: InsertRow<Table = T>> Insert<D, T, R> {
    /// Bulk insert: add another row to the same statement.
    pub fn values(mut self, row: R) -> Self {
        self.rows.push(row.into_values());
        self
    }
}

impl<D: SupportsOnConflict, T: Table, R: InsertRow<Table = T>> Insert<D, T, R> {
    /// `ON CONFLICT (..) DO NOTHING`.
    pub fn on_conflict_do_nothing(mut self, target: impl ConflictTarget<T>) -> Self {
        self.on_conflict = Some(ConflictClause {
            target: target.column_names(),
            action: ConflictAction::DoNothing,
        });
        self
    }

    /// `ON CONFLICT (..) DO UPDATE SET ..`, reusing the same `*Update`
    /// struct `update().set(..)` takes.
    pub fn on_conflict_do_update<U: UpdateRow<Table = T>>(
        mut self,
        target: impl ConflictTarget<T>,
        set: U,
    ) -> Self {
        self.on_conflict = Some(ConflictClause {
            target: target.column_names(),
            action: ConflictAction::DoUpdate(set.sets()),
        });
        self
    }
}

impl<D: Dialect, T: Table, R: InsertRow<Table = T>> Insert<D, T, R> {
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        render_values_clause::<D, T, R>(&self.rows, &self.on_conflict)
    }
}

impl<D: SupportsReturning, T: Table, R: InsertRow<Table = T>> Insert<D, T, R> {
    /// Returns a distinct `InsertReturning` type rather than `Self` with a
    /// flag set: the execution layer needs `Sel`'s concrete type to know what
    /// to decode a returned row into, and an optional field would erase
    /// it.
    pub fn returning<Sel, Idx>(self, sel: Sel) -> InsertReturning<D, T, R, Sel>
    where
        Sel: Selection<Cons<TableSlot<T, NotNull>, Nil>, Idx>,
    {
        InsertReturning {
            rows: self.rows,
            on_conflict: self.on_conflict,
            returning: sel.items(),
            _marker: PhantomData,
        }
    }
}

/// An `INSERT .. RETURNING ..` statement. `Sel`'s concrete type is retained
/// (unlike a hypothetical `Option<Vec<SelectItem>>` field on `Insert` itself)
/// specifically so the execution layer can decode returned rows into
/// `Sel::Output` — see `Insert::returning`'s doc comment.
pub struct InsertReturning<D, T: Table, R: InsertRow<Table = T>, Sel> {
    rows: Vec<Vec<InsertValue>>,
    on_conflict: Option<ConflictClause>,
    returning: Vec<SelectItem>,
    _marker: PhantomData<fn() -> (D, T, R, Sel)>,
}

impl<D: Dialect, T: Table, R: InsertRow<Table = T>, Sel> InsertReturning<D, T, R, Sel> {
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        let (mut sql, mut params) = render_values_clause::<D, T, R>(&self.rows, &self.on_conflict);
        sql.push_str(" RETURNING ");
        render_select_list::<D>(&self.returning, &mut sql, &mut params);
        (sql, params)
    }
}
