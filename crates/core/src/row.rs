//! Rows keyed by column rather than by position.
//!
//! A tuple selection decodes to `Row<..>`: a type-level list of
//! `(key, value)` cells. A column's key is its `expr::ColumnKey`, a computed
//! expression's is the identity it carries (`expr::Count`,
//! `window::RowNumber`), and `.label(label::..)` supplies one for anything that has
//! none. `Field` looks a key up the way `scope::Find` looks a table up, with
//! the same `Here`/`There` index.
//!
//! Keying by column is what makes adding a column to a selection a
//! non-breaking change, and what makes two same-typed columns
//! (`orders::user_id` and `orders::total` are both `BigInt`) impossible to
//! transpose. `into_tuple`/`into_tuples` recover the positional view where
//! destructuring is what's wanted, and `FromRow` fills a plain struct by
//! matching field *names*, so a DTO names no column and no table.
//!
//! Naming a row type takes a type alias long enough to trip
//! `clippy::type_complexity`, the same lint `qbrs-core` allows crate-wide.
//! Inference covers every use that stays inside a function.
//!
//! **Known limitations**: a key selected twice is ambiguous at the point it
//! is read, rather than resolving to the first — give one of them a
//! `label!{}` label. `into_tuple` is implemented up to 16 columns; `Row`
//! itself has no such limit. A field with no name (a bare `sql!{}`
//! fragment) can only be reached positionally until `.label(label::..)` gives it one.

use std::marker::PhantomData;

use crate::expr::{Column, ColumnKey, Keyed, Labeled, SqlType};
use crate::scope::{Cons, Here, Nil, There};

/// The empty row.
pub struct RowNil;

/// One field: `V`, filed under key `K`, followed by the rest in `Tail`.
pub struct RowCons<K, V, Tail> {
    value: V,
    tail: Tail,
    _key: PhantomData<fn() -> K>,
}

impl<K, V, Tail> RowCons<K, V, Tail> {
    /// This cell's value. With `tail` and the key's `Named::NAME`, this is
    /// everything a downstream crate needs to walk a row under whatever
    /// bounds it wants — `serde::Serialize`, `Display`, anything — which
    /// `qbrs-core` can't offer itself, having no dependencies.
    pub fn value(&self) -> &V {
        &self.value
    }

    /// The rest of the row.
    pub fn tail(&self) -> &Tail {
        &self.tail
    }

    #[doc(hidden)]
    pub fn new(value: V, tail: Tail) -> Self {
        RowCons {
            value,
            tail,
            _key: PhantomData,
        }
    }
}

/// Proof that a row holds a field under key `K`, at compile-time-inferred
/// position `Idx`. `Idx` is never spelled out by callers, exactly as in
/// `scope::Find`, and is what keeps the two impls below structurally
/// distinct rather than overlapping.
///
/// Borrowing and moving are one trait because they are one search: `pluck`
/// additionally reports what the row is left holding, so several fields can
/// be moved out in turn.
#[diagnostic::on_unimplemented(
    message = "`{K}` is not in this query's selection",
    label = "a row can only be read by a key the query selected",
    note = "add `{K}` to the query's selection list, or `.label(label::..)` the expression you meant — and in a generic helper give each column its own `Idx` parameter, since one shared index matches no row"
)]
pub trait Field<K, Idx> {
    type Value;
    type Rest;
    fn peek(&self) -> &Self::Value;
    fn pluck(self) -> (Self::Value, Self::Rest);
}

impl<K, V, Tail> Field<K, Here> for RowCons<K, V, Tail> {
    type Value = V;
    type Rest = Tail;
    fn peek(&self) -> &V {
        &self.value
    }
    fn pluck(self) -> (V, Tail) {
        (self.value, self.tail)
    }
}

#[diagnostic::do_not_recommend]
impl<K, Other, V, Tail, I> Field<K, There<I>> for RowCons<Other, V, Tail>
where
    Tail: Field<K, I>,
{
    type Value = <Tail as Field<K, I>>::Value;
    type Rest = RowCons<Other, V, <Tail as Field<K, I>>::Rest>;
    fn peek(&self) -> &Self::Value {
        self.tail.peek()
    }
    fn pluck(self) -> (Self::Value, Self::Rest) {
        let (value, rest) = self.tail.pluck();
        (value, RowCons::new(self.value, rest))
    }
}

/// An identifier spelled one `char` per cell, so two keys declared in
/// different crates can be compared by the *name* they share rather than by
/// being the same type. `char` is one of the three types stable const
/// generics accept.
#[doc(hidden)]
pub struct NameChar<const C: char, Rest>(PhantomData<Rest>);

/// End of a `NameChar` chain.
#[doc(hidden)]
pub struct NameEnd;

/// Builds a `NameChar` chain from character literals.
#[doc(hidden)]
#[macro_export]
macro_rules! type_name {
    () => { $crate::row::NameEnd };
    ($c:literal $(, $rest:literal)*) => {
        $crate::row::NameChar<$c, $crate::type_name!($($rest),*)>
    };
}

/// A key that has a name, so a field can be found by what it is called
/// rather than by which key type produced it, and so a row can print itself
/// keyed. Implemented by `#[derive(Table)]` for columns, by `label!` for
/// labels, and by the built-in expression keys.
pub trait Named: named::Sealed {
    type Name;
    const NAME: &'static str;
}

pub(crate) mod named {
    /// Sealed the way `scope::BaseTable` is: a name is written by a macro —
    /// `#[derive(Table)]`, `with!`, `label!`, or `expr_key!` — so the
    /// spelling in `Named::NAME` and the one in the SQL cannot disagree.
    pub trait Sealed {}
}

#[doc(hidden)]
pub use named::Sealed as NamedSealed;

/// A key someone wrote down: a column, a `label!`, or one of the built-in
/// expression keys. `Anon` is deliberately not one, which is what keeps two
/// unnamed columns from standing in for each other, keeps an unnamed field
/// out of reach of `.get()`, and keeps a by-name lookup from landing on one.
pub trait Spelled: Named {}

/// The key of a selected item that carries no name of its own — a bare
/// `sql!{}` fragment.
pub struct Anon;

#[doc(hidden)]
impl NamedSealed for Anon {}

impl Named for Anon {
    type Name = NameEnd;
    const NAME: &'static str = "?";
}

/// `Field` by name rather than by key identity, which is what lets a struct
/// that has never heard of `users::email` still receive it.
#[diagnostic::on_unimplemented(
    message = "this query's rows have no field matching `{F}`",
    label = "the selection needs a column of that name, decoding to that type",
    note = "a computed expression is matched by name only once `.label(label::..)` gives it one"
)]
pub trait TakeNamed<F, Idx> {
    type Value;
    type Rest;
    fn take_named(self) -> (Self::Value, Self::Rest);
}

impl<F, K, V, Tail> TakeNamed<F, Here> for RowCons<K, V, Tail>
where
    K: Spelled,
    F: Spelled<Name = <K as Named>::Name>,
{
    type Value = V;
    type Rest = Tail;
    fn take_named(self) -> (V, Tail) {
        (self.value, self.tail)
    }
}

#[diagnostic::do_not_recommend]
impl<F, K, V, Tail, I> TakeNamed<F, There<I>> for RowCons<K, V, Tail>
where
    Tail: TakeNamed<F, I>,
{
    type Value = <Tail as TakeNamed<F, I>>::Value;
    type Rest = RowCons<K, V, <Tail as TakeNamed<F, I>>::Rest>;
    fn take_named(self) -> (Self::Value, Self::Rest) {
        let (value, rest) = self.tail.take_named();
        (value, RowCons::new(self.value, rest))
    }
}

/// A row's keys as a type-level list, so two rows can be checked against
/// each other where only their shape is in play — a CTE body against its
/// declared columns, or one `UNION` branch against another.
pub trait RowKeys {
    type Keys;
}

impl RowKeys for RowNil {
    type Keys = Nil;
}

impl<K, V, Tail: RowKeys> RowKeys for RowCons<K, V, Tail> {
    type Keys = Cons<K, Tail::Keys>;
}

impl<L: RowKeys> RowKeys for Row<L> {
    type Keys = L::Keys;
}

/// One column can stand in for another: they are called the same thing.
#[diagnostic::on_unimplemented(
    message = "column `{Self}` can't stand in for `{Other}`",
    label = "these two columns must have the same name",
    note = "matched by name: `.label(label::..)` whichever side is spelled wrong — and an unnamed expression (`Anon`) has no name to match with at all"
)]
pub trait SameNameAs<Other> {}

#[diagnostic::do_not_recommend]
impl<A, B> SameNameAs<B> for A
where
    A: Spelled,
    B: Spelled<Name = <A as Named>::Name>,
{
}

/// Two key lists name the same columns, in the same order. Values are
/// checked separately, by `RowValues`; this is what stops a body whose
/// columns are merely type-compatible from being spliced in transposed.
#[diagnostic::on_unimplemented(
    message = "these columns don't line up by name",
    label = "each column must have the same name, in the same order, as the one it stands in for"
)]
pub trait SameNames<Other> {}

impl SameNames<Nil> for Nil {}

#[diagnostic::do_not_recommend]
impl<A, B, TailA, TailB> SameNames<Cons<B, TailB>> for Cons<A, TailA>
where
    A: SameNameAs<B>,
    TailA: SameNames<TailB>,
{
}

/// Two selections produce the same row: the same column names, in the same
/// order, decoding to the same types. A one-column selection decodes to a
/// bare value rather than a `Row`, and two of those match when the value
/// types do — there is no name to disagree about. Names as well as types, because a
/// `UNION` branch or a CTE body whose columns merely happen to be
/// type-compatible would otherwise splice in transposed.
#[diagnostic::on_unimplemented(
    message = "these two selections don't produce the same row",
    label = "must select the same column names, in the same order, decoding to the same types"
)]
pub trait SameShape<Other> {}

/// The values half of `SameShape`, split out so a mismatch reports itself
/// rather than surfacing as an associated-type equality failure inside the
/// blanket impl below.
#[diagnostic::on_unimplemented(
    message = "these two selections don't decode to the same values",
    label = "the same types, in the same order, are needed on both sides"
)]
pub trait SameValues<Other> {}

#[diagnostic::do_not_recommend]
impl<A: RowValues, B: RowValues<Values = <A as RowValues>::Values>> SameValues<B> for A {}

impl<A, B> SameShape<Row<B>> for Row<A>
where
    Row<A>: SameValues<Row<B>> + RowKeys,
    Row<B>: RowKeys,
    <Row<A> as RowKeys>::Keys: SameNames<<Row<B> as RowKeys>::Keys>,
{
}

/// Maps a value written in a selection list to the type its field is filed
/// under, so a field is read back with the same value that selected it.
pub trait RowKey {
    type Key;
}

impl<C: ColumnKey> RowKey for Column<C> {
    type Key = C;
}

impl<K, Req, S: SqlType> RowKey for Keyed<K, Req, S> {
    type Key = K;
}

impl<K, Inner> RowKey for Labeled<K, Inner> {
    type Key = K;
}

impl<Req, S: SqlType> RowKey for crate::expr::Expr<Req, S> {
    type Key = Anon;
}

/// A value that can name a field at a `.get()`/`.take()` call. Every
/// `RowKey` can *file* a field; only these can find one again, which is what
/// keeps an unlabelled expression's `Anon` field out of reach of any other
/// unlabelled expression.
#[diagnostic::on_unimplemented(
    message = "`{Self}` doesn't name a field",
    label = "an unlabelled expression has no name to look up",
    note = "give it one with `.label(label::..)`, or read it positionally with `into_tuple()`"
)]
pub trait LookupKey: RowKey {}

#[diagnostic::do_not_recommend]
impl<C: ColumnKey> LookupKey for Column<C> {}
#[diagnostic::do_not_recommend]
impl<K: Spelled, Req, S: SqlType> LookupKey for Keyed<K, Req, S> {}
#[diagnostic::do_not_recommend]
impl<K: Spelled, Inner> LookupKey for Labeled<K, Inner> {}

/// A decoded row. Its fields are fixed by the query's selection list, and
/// each is read by the same value that selected it.
pub struct Row<L>(L);

impl<L> Row<L> {
    #[doc(hidden)]
    pub fn new(fields: L) -> Self {
        Row(fields)
    }

    /// The row's fields as a `RowCons` chain, for walking it from another
    /// crate. `get`/`take`/`into_struct` cover reading a known field; this
    /// is for code that has to visit every field it happens to hold.
    pub fn fields(&self) -> &L {
        &self.0
    }

    /// `row.get(users::email)` — the key is the same value that appeared in
    /// the selection list, so there is no name to keep in sync and no
    /// position to get wrong.
    pub fn get<K: LookupKey, Idx>(&self, _key: K) -> &<L as Field<K::Key, Idx>>::Value
    where
        L: Field<K::Key, Idx>,
    {
        self.0.peek()
    }

    /// Moves one field out and hands back the row without it, so several
    /// fields can be taken in turn.
    pub fn take<K: LookupKey, Idx>(
        self,
        _key: K,
    ) -> (
        <L as Field<K::Key, Idx>>::Value,
        Row<<L as Field<K::Key, Idx>>::Rest>,
    )
    where
        L: Field<K::Key, Idx>,
    {
        let (value, rest) = self.0.pluck();
        (value, Row::new(rest))
    }

    /// Reads a field by naming its key type rather than passing the value
    /// that selected it — what the generated accessors use, since a
    /// built-in expression key is never spelled at a call site.
    #[doc(hidden)]
    pub fn peek_key<K, Idx>(&self) -> &<L as Field<K, Idx>>::Value
    where
        L: Field<K, Idx>,
    {
        self.0.peek()
    }

    #[doc(hidden)]
    pub fn take_named<F, Idx>(
        self,
    ) -> (
        <L as TakeNamed<F, Idx>>::Value,
        Row<<L as TakeNamed<F, Idx>>::Rest>,
    )
    where
        L: TakeNamed<F, Idx>,
    {
        let (value, rest) = self.0.take_named();
        (value, Row::new(rest))
    }

    /// Builds a `#[derive(FromRow)]` struct out of this row, matching its
    /// fields by name. Extra columns in the row are ignored, and the order
    /// they were selected in doesn't matter.
    pub fn into_struct<T, Idxs>(self) -> T
    where
        T: FromRow<L, Idxs>,
    {
        T::from_row(self)
    }

    /// The positional view: the plain tuple this selection would decode to
    /// if rows didn't exist.
    pub fn into_tuple(self) -> L::Values
    where
        L: RowValues,
    {
        self.0.into_values()
    }
}

/// Prints a row keyed, since being keyed is the whole point of the type.
pub trait DebugFields {
    fn fmt_fields(&self, f: &mut std::fmt::DebugStruct<'_, '_>);
}

impl DebugFields for RowNil {
    fn fmt_fields(&self, _f: &mut std::fmt::DebugStruct<'_, '_>) {}
}

impl<K: Named, V: std::fmt::Debug, Tail: DebugFields> DebugFields for RowCons<K, V, Tail> {
    fn fmt_fields(&self, f: &mut std::fmt::DebugStruct<'_, '_>) {
        f.field(K::NAME, &self.value);
        self.tail.fmt_fields(f);
    }
}

impl<L: DebugFields> std::fmt::Debug for Row<L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("Row");
        self.0.fmt_fields(&mut s);
        s.finish()
    }
}

impl<K, V: Clone, Tail: Clone> Clone for RowCons<K, V, Tail> {
    fn clone(&self) -> Self {
        RowCons::new(self.value.clone(), self.tail.clone())
    }
}

impl Clone for RowNil {
    fn clone(&self) -> Self {
        RowNil
    }
}

impl<L: Clone> Clone for Row<L> {
    fn clone(&self) -> Self {
        Row(self.0.clone())
    }
}

impl<K, V: PartialEq, Tail: PartialEq> PartialEq for RowCons<K, V, Tail> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.tail == other.tail
    }
}

impl PartialEq for RowNil {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<K, V: Eq, Tail: Eq> Eq for RowCons<K, V, Tail> {}
impl Eq for RowNil {}

impl<L: PartialEq> PartialEq for Row<L> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<L: Eq> Eq for Row<L> {}

/// A row's fields as a plain tuple, in selection order.
pub trait RowValues {
    type Values;
    fn into_values(self) -> Self::Values;
}

/// Adds one element to the front of a tuple. The only place `Row`'s
/// positional view has an arity limit.
#[diagnostic::on_unimplemented(
    message = "this row has no positional view",
    label = "`into_tuple`/`into_tuples` stop at 16 fields, however they were selected",
    note = "read it by key (`row.get(..)`) or fill a struct with `#[derive(FromRow)]`"
)]
pub trait Prepend<H> {
    type Output;
    fn prepend(self, head: H) -> Self::Output;
}

impl<H> Prepend<H> for () {
    type Output = (H,);
    fn prepend(self, head: H) -> (H,) {
        (head,)
    }
}

macro_rules! prepend_impls {
    () => {};
    ($first:ident $(, $rest:ident)*) => {
        #[allow(non_snake_case)]
        impl<H, $first $(, $rest)*> Prepend<H> for ($first, $($rest,)*) {
            type Output = (H, $first, $($rest,)*);
            fn prepend(self, head: H) -> Self::Output {
                let ($first, $($rest,)*) = self;
                (head, $first, $($rest,)*)
            }
        }
        prepend_impls!($($rest),*);
    };
}
prepend_impls!(
    T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15
);

impl RowValues for RowNil {
    type Values = ();
    fn into_values(self) {}
}

impl<K, V, Tail> RowValues for RowCons<K, V, Tail>
where
    Tail: RowValues,
    Tail::Values: Prepend<V>,
{
    type Values = <Tail::Values as Prepend<V>>::Output;
    fn into_values(self) -> Self::Values {
        self.tail.into_values().prepend(self.value)
    }
}

impl<L: RowValues> RowValues for Row<L> {
    type Values = L::Values;
    fn into_values(self) -> Self::Values {
        self.0.into_values()
    }
}

/// `Vec<Row<..>> -> Vec<(..)>`.
pub trait IntoTuples {
    type Tuples;
    fn into_tuples(self) -> Self::Tuples;
}

impl<L: RowValues> IntoTuples for Vec<Row<L>> {
    type Tuples = Vec<L::Values>;
    fn into_tuples(self) -> Vec<L::Values> {
        self.into_iter().map(Row::into_tuple).collect()
    }
}

/// Builds a plain struct out of a row by matching field names, generated by
/// `#[derive(FromRow)]`. `Idxs` holds the per-field lookup indices, for the
/// reason `scope::Superset` explains, which is also why this is its own
/// trait rather than `From`.
pub trait FromRow<L, Idxs>: Sized {
    fn from_row(row: Row<L>) -> Self;
}

/// `Vec<Row<..>> -> Vec<T>` for any `#[derive(FromRow)]` struct.
pub trait IntoStructs {
    type Fields;
    fn into_structs<T, Idxs>(self) -> Vec<T>
    where
        T: FromRow<Self::Fields, Idxs>;
}

impl<L> IntoStructs for Vec<Row<L>> {
    type Fields = L;
    fn into_structs<T, Idxs>(self) -> Vec<T>
    where
        T: FromRow<L, Idxs>,
    {
        self.into_iter().map(Row::into_struct).collect()
    }
}

/// Declares an expression's own row key and the `row.<name>()` accessor that
/// reads it, so a selection using one needs nothing declared at the call
/// site.
macro_rules! expr_key {
    ($key:ident, $accessor:ident, $method:ident, $doc:literal, $($ch:literal),+) => {
        #[doc = $doc]
        #[derive(Clone, Copy)]
        pub struct $key;

        #[doc(hidden)]
        impl $crate::row::NamedSealed for $key {}

        #[doc(hidden)]
        impl $crate::row::Named for $key {
            type Name = $crate::type_name!($($ch),+);
            const NAME: &'static str = concat!($($ch),+);
        }

        #[doc(hidden)]
        impl $crate::row::Spelled for $key {}

        #[doc = $doc]
        pub trait $accessor<Idx> {
            type Value;
            fn $method(&self) -> &Self::Value;
        }

        impl<L, Idx> $accessor<Idx> for $crate::row::Row<L>
        where
            L: $crate::row::Field<$key, Idx>,
        {
            type Value = <L as $crate::row::Field<$key, Idx>>::Value;
            fn $method(&self) -> &Self::Value {
                self.peek_key::<$key, Idx>()
            }
        }
    };
}
pub(crate) use expr_key;
