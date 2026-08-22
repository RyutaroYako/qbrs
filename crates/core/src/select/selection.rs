//! What a `.select(..)` list decodes to once a row comes back.

use crate::expr::{Column, ColumnKey, Expr, ExprKind, Keyed, LabelKey, Labeled, SqlType};
use crate::render::SelectItem;
use std::marker::PhantomData;

use crate::row::{Named, Row, RowCons, RowKey, RowNil};
use crate::scope::{Find, Nil, Superset, Table, WrapNullable};

/// One *field* of a resulting `Row`: the key its value is filed under, and
/// the Rust type it decodes to. A selection list is a chain of
/// `SelectionPart`s, one of which — `All` — carries many of these at once.
///
/// Parameterized by `Scope` so a bare column's `Value` is `Option<T>` when —
/// and only when — that column's table is nullable in *this* query, via
/// `scope::Find::Nullability` + `WrapNullable`. Nullability is therefore
/// derived from join shape rather than asserted with a manual `.nullable()`.
///
/// Scope membership is proven as a side effect of this trait type-checking
/// at all, through the `Find`/`Superset` bounds below — so callers need no
/// separate check.
#[diagnostic::on_unimplemented(
    message = "`{Self}` can't be a field of this query's rows",
    label = "columns, aggregates, window functions and scope-free `sql!` fragments can be",
    note = "a fragment that names a table has to say what it decodes to first — `.decodes_as::<Nullable<BigInt>>()` — since its NULL-ability doesn't follow from any one column's join"
)]
pub trait RowField<Scope, Idx>: RowKey {
    type Value;
    fn item(&self) -> SelectItem;
}

impl<C: ColumnKey, Scope, Idx> RowField<Scope, Idx> for Column<C>
where
    Scope: Find<C::Table, Idx>,
    C::Sql: WrapNullable<<Scope as Find<C::Table, Idx>>::Nullability>,
    <C::Sql as WrapNullable<<Scope as Find<C::Table, Idx>>::Nullability>>::Output: SqlType,
{
    type Value = <<C::Sql as WrapNullable<
        <Scope as Find<C::Table, Idx>>::Nullability,
    >>::Output as SqlType>::Native;
    fn item(&self) -> SelectItem {
        SelectItem::bare(ExprKind::Column {
            table: <C::Table as Table>::NAME,
            name: C::NAME,
        })
    }
}

// Only a scope-free expression is selectable as a bare `Expr`. One that
// names a table has a per-query nullability that `S` doesn't carry, so
// selecting it would report the declared type where a `LEFT JOIN` produces
// NULL; a `Column` is how a table's value is selected, and `sql!{}` — always
// `Nil` — is how a computed one is.
impl<S: SqlType, Scope, Idx> RowField<Scope, Idx> for Expr<Nil, S>
where
    Scope: Superset<Nil, Idx>,
{
    type Value = S::Native;
    fn item(&self) -> SelectItem {
        SelectItem::bare(self.kind.clone())
    }
}

impl<K, Req, S: SqlType, Scope, Idx> RowField<Scope, Idx> for Keyed<K, Req, S>
where
    Scope: Superset<Req, Idx>,
{
    type Value = S::Native;
    fn item(&self) -> SelectItem {
        SelectItem::bare(self.kind.clone())
    }
}

/// A label renames whatever it wraps and changes nothing else, so this is
/// one impl rather than one per selectable: the inner value decides the
/// scope check and the decoded type, the label decides the `AS` and the row
/// key.
impl<K: LabelKey, Inner: RowField<Scope, Idx>, Scope, Idx> RowField<Scope, Idx>
    for Labeled<K, Inner>
{
    type Value = Inner::Value;
    fn item(&self) -> SelectItem {
        SelectItem::labeled(self.inner.item().kind, <K as Named>::NAME)
    }
}

/// A whole `SELECT` list. A tuple decodes to a `row::Row` keyed by each
/// element's `RowField::Key`; a single un-tupled element decodes to its bare
/// value, since there is nothing to key it against.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a valid selection list here",
    label = "a selection is a column, an expression, `<table>::All`, or a tuple of up to 16 of those",
    note = "every element has to be in scope — `.from(..)`/`.join(..)` the tables it names — and a `sql!` fragment that names one has to state its decoded type with `.decodes_as::<..>()`"
)]
pub trait Selection<Scope, Idx> {
    type Output;
    fn items(&self) -> Vec<SelectItem>;
}

macro_rules! scalar_selection {
    (impl[$($generics:tt)*] $ty:ty) => {
        impl<$($generics)*, Scope, Idx> Selection<Scope, Idx> for $ty
        where
            $ty: RowField<Scope, Idx>,
        {
            type Output = <$ty as RowField<Scope, Idx>>::Value;
            fn items(&self) -> Vec<SelectItem> {
                vec![RowField::item(self)]
            }
        }
    };
}

/// One element of a selection list. A column or an expression contributes
/// one field; `All` contributes a whole table's worth. `Fields<Tail>` is
/// what it puts in front of whatever the rest of the list contributes, so a
/// list is assembled by nesting rather than by concatenating afterwards.
#[diagnostic::on_unimplemented(
    message = "`{Self}` can't be part of a selection list",
    label = "columns, aggregates, window functions, `<table>::All` and scope-free `sql!` fragments can be",
    note = "a fragment that names a table has to say what it decodes to first — `.decodes_as::<Nullable<BigInt>>()` — since its NULL-ability doesn't follow from any one column's join"
)]
pub trait SelectionPart<Scope, Idx> {
    type Fields<Tail>;
    fn push_items(&self, out: &mut Vec<SelectItem>);
}

macro_rules! field_part {
    (impl[$($generics:tt)*] $ty:ty) => {
        impl<$($generics)*, Scope, Idx> SelectionPart<Scope, Idx> for $ty
        where
            $ty: RowField<Scope, Idx>,
        {
            type Fields<Tail> = RowCons<
                <$ty as RowKey>::Key,
                <$ty as RowField<Scope, Idx>>::Value,
                Tail,
            >;
            fn push_items(&self, out: &mut Vec<SelectItem>) {
                out.push(RowField::item(self));
            }
        }
    };
}
/// Everything selectable on its own: as a whole list of one, and as one
/// part of a longer list. Stated once, since a selectable that is one and
/// not the other has never been a thing.
macro_rules! selectable {
    (impl[$($generics:tt)*] $ty:ty) => {
        scalar_selection!(impl[$($generics)*] $ty);
        field_part!(impl[$($generics)*] $ty);
    };
}
selectable!(impl[C: ColumnKey] Column<C>);
selectable!(impl[S: SqlType] Expr<Nil, S>);
selectable!(impl[K, Req, S: SqlType] Keyed<K, Req, S>);
selectable!(impl[K, Inner] Labeled<K, Inner>);

/// Every column of one table, in declaration order — `select(users::All)`.
/// The table's own `#[derive(Table)]` supplies the chain through
/// `AllColumns`, so a selection list and the schema cannot drift apart, and
/// a whole table counts as one element of a tuple however many columns it
/// has.
pub struct All<T>(PhantomData<fn() -> T>);

impl<T> All<T> {
    pub const fn new() -> Self {
        All(PhantomData)
    }
}

impl<T> Clone for All<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for All<T> {}

impl<T> Default for All<T> {
    fn default() -> Self {
        All::new()
    }
}

/// What `#[derive(Table)]` emits so `All<Table>` knows the table's columns
/// and what each of them decodes to in a given scope.
pub trait AllColumns<Scope, Idx> {
    type Fields<Tail>;
    fn push_items(out: &mut Vec<SelectItem>);
}

impl<T: AllColumns<Scope, Idx>, Scope, Idx> SelectionPart<Scope, Idx> for All<T> {
    type Fields<Tail> = T::Fields<Tail>;
    fn push_items(&self, out: &mut Vec<SelectItem>) {
        T::push_items(out);
    }
}

impl<T: AllColumns<Scope, Idx>, Scope, Idx> Selection<Scope, Idx> for All<T> {
    type Output = Row<T::Fields<RowNil>>;
    fn items(&self) -> Vec<SelectItem> {
        let mut out = Vec::new();
        T::push_items(&mut out);
        out
    }
}

macro_rules! row_chain {
    ($n:ident $i:ident) => {
        <$n as SelectionPart<Scope, $i>>::Fields<RowNil>
    };
    ($n:ident $i:ident, $($rest:tt)*) => {
        <$n as SelectionPart<Scope, $i>>::Fields<row_chain!($($rest)*)>
    };
}

macro_rules! tuple_selection {
    ($($n:ident $i:ident),+) => {
        #[allow(non_snake_case)]
        impl<Scope, $($n,)+ $($i,)+> Selection<Scope, ($($i,)+)> for ($($n,)+)
        where
            $($n: SelectionPart<Scope, $i>,)+
        {
            type Output = Row<row_chain!($($n $i),+)>;
            fn items(&self) -> Vec<SelectItem> {
                let ($($n,)+) = self;
                let mut out = Vec::new();
                $(SelectionPart::push_items($n, &mut out);)+
                out
            }
        }
    };
}
tuple_selection!(A IA);
tuple_selection!(A IA, B IB);
tuple_selection!(A IA, B IB, C IC);
tuple_selection!(A IA, B IB, C IC, D ID);
tuple_selection!(A IA, B IB, C IC, D ID, E IE);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP);
