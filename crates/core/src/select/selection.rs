//! What a `.select(..)` list decodes to once a row comes back.

use crate::expr::{Column, ColumnKey, ExprKind, Keyed, LabelKey, Labeled, SqlType};
use crate::render::SelectItem;
use std::marker::PhantomData;

use crate::row::{Named, Row, RowCons, RowKey, RowNil};
use crate::scope::{Find, Superset, Table, WrapNullable};

/// Sealed for the reason `InsertRow` is: these traits pair a type-level
/// claim (`Fields`/`Output`) with the runtime list of `SelectItem`s that is
/// supposed to match it, and only the impls in this crate keep the two in
/// step. A hand-written one could select a row that decodes transposed —
/// the failure `row::SameShape` and column-keyed rows exist to stop.
mod private {
    /// Carries the trait's own parameters, for the reason `scope::proof`
    /// explains: `Self` can be an honest `Column<C>` while the free `Idx`
    /// is the caller's own type, so a seal on `Self` alone admits the
    /// forgery — and a private proof *type* is reachable by projection.
    pub trait Sealed<Scope, Idx> {}
}

/// `AllColumns`/`CteShape` are emitted in the schema's own crate, so their
/// seal has to be nameable there — a separate trait, because sharing
/// `private::Sealed` would hand out the one line that unseals `Selection`
/// too, and a hand-written `Selection` is exactly what the seal is for.
#[doc(hidden)]
pub trait SelectableSealed {}

impl<C, Scope, Idx> private::Sealed<Scope, Idx> for Column<C>
where
    C: crate::expr::ColumnKey,
    Scope: Find<C::Table, Idx>,
{
}

impl<K, Req, S: SqlType, Scope, Idx> private::Sealed<Scope, Idx> for Keyed<K, Req, S> where
    Scope: Superset<Req, Idx>
{
}

impl<K, Inner, Scope, Idx> private::Sealed<Scope, Idx> for Labeled<K, Inner> where
    Inner: RowField<Scope, Idx>
{
}

impl<T: AllColumns, Scope, Idx> private::Sealed<Scope, Idx> for All<T> where
    T::Columns: ColumnList<Scope, Idx>
{
}

/// What a single un-tupled selection decodes to: a bare native value, or
/// its `Option`. Sealed by construction — the impls come from the same
/// `sql_leaf_type!` that declares the types — and used to give a
/// one-column set operation an `ORDER BY` with no position to state.
pub trait SingleColumn {}

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
    label = "a column, an aggregate, a window function, a `sql!` fragment, or a labelled one of those can be",
    note = "an expression the builder inferred a type for — a comparison, an `is_null`, a `LIKE` — has to state what it decodes to with `.decodes_as::<..>()`, since that inference can contradict the join; a `sql!` fragment already states it"
)]
pub trait RowField<Scope, Idx>: RowKey + private::Sealed<Scope, Idx> {
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
    label = "a selection is a column, an aggregate, a window function, a `sql!` fragment, a labelled one of those, `<table>::All`, or a tuple of up to 32 of them",
    note = "every element has to be in scope — `.from(..)`/`.join(..)` the tables it names — and an expression the builder inferred a type for has to state its decoded type with `.decodes_as::<..>()`"
)]
pub trait Selection<Scope, Idx>: private::Sealed<Scope, Idx> {
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
    label = "a column, an aggregate, a window function, a `sql!` fragment, a labelled one of those, or `<table>::All` can be",
    note = "an expression the builder inferred a type for — a comparison, an `is_null`, a `LIKE` — has to state what it decodes to with `.decodes_as::<..>()`, since that inference can contradict the join"
)]
pub trait SelectionPart<Scope, Idx>: private::Sealed<Scope, Idx> {
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
pub trait AllColumns: SelectableSealed {
    /// The table's columns as a type-level list, `Cons<Column<C>, ..>`.
    /// The row and the rendered items are both computed from it here, so a
    /// hand-written impl can name a different set of columns but can never
    /// make the two disagree — which is what a schema's own crate could do
    /// while this trait stated the row and pushed the items separately.
    type Columns;
}

/// The list `AllColumns` names, walked once for the row's fields and once
/// for the items. Implemented for `Nil` and `Cons<Column<C>, Tail>` only,
/// and only here — sealed, because this trait *is* the pairing `AllColumns`
/// was split up to remove: it states the row and pushes the items
/// separately, so a hand-written impl could transpose them. Nothing outside
/// this crate implements it, so an ordinary private supertrait is enough;
/// no `Proof` is needed.
mod column_list {
    pub trait Sealed {}
    impl Sealed for crate::scope::Nil {}
    impl<C: crate::expr::ColumnKey, Tail> Sealed for crate::scope::Cons<crate::expr::Column<C>, Tail> {}
}

pub trait ColumnList<Scope, Idx>: column_list::Sealed {
    type Fields<Tail>;
    fn push_items(out: &mut Vec<SelectItem>);
}

impl<Scope, Idx> ColumnList<Scope, Idx> for crate::scope::Nil {
    type Fields<Tail> = Tail;
    fn push_items(_out: &mut Vec<SelectItem>) {}
}

impl<C: ColumnKey, Tail, Scope, Idx> ColumnList<Scope, Idx> for crate::scope::Cons<Column<C>, Tail>
where
    Column<C>: RowField<Scope, Idx>,
    Tail: ColumnList<Scope, Idx>,
{
    type Fields<T> = RowCons<C, <Column<C> as RowField<Scope, Idx>>::Value, Tail::Fields<T>>;
    fn push_items(out: &mut Vec<SelectItem>) {
        out.push(RowField::item(&Column::<C>::new()));
        Tail::push_items(out);
    }
}

impl<T: AllColumns, Scope, Idx> SelectionPart<Scope, Idx> for All<T>
where
    T::Columns: ColumnList<Scope, Idx>,
{
    type Fields<Tail> = <T::Columns as ColumnList<Scope, Idx>>::Fields<Tail>;
    fn push_items(&self, out: &mut Vec<SelectItem>) {
        <T::Columns as ColumnList<Scope, Idx>>::push_items(out);
    }
}

impl<T: AllColumns, Scope, Idx> Selection<Scope, Idx> for All<T>
where
    T::Columns: ColumnList<Scope, Idx>,
{
    type Output = Row<<T::Columns as ColumnList<Scope, Idx>>::Fields<RowNil>>;
    fn items(&self) -> Vec<SelectItem> {
        let mut out = Vec::new();
        <T::Columns as ColumnList<Scope, Idx>>::push_items(&mut out);
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
        impl<Scope, $($n,)+ $($i,)+> private::Sealed<Scope, ($($i,)+)> for ($($n,)+)
        where
            $($n: SelectionPart<Scope, $i>,)+
        {
        }

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
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW, X IX);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW, X IX, Y IY);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW, X IX, Y IY, Z IZ);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW, X IX, Y IY, Z IZ, AA IAA);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW, X IX, Y IY, Z IZ, AA IAA, BB IBB);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW, X IX, Y IY, Z IZ, AA IAA, BB IBB, CC ICC);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW, X IX, Y IY, Z IZ, AA IAA, BB IBB, CC ICC, DD IDD);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW, X IX, Y IY, Z IZ, AA IAA, BB IBB, CC ICC, DD IDD, EE IEE);
tuple_selection!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP, Q IQ, R IR, S IS, T IT, U IU, V IV, W IW, X IX, Y IY, Z IZ, AA IAA, BB IBB, CC ICC, DD IDD, EE IEE, FF IFF);
