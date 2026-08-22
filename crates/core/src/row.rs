//! Rows keyed by column rather than by position.
//!
//! A tuple selection decodes to `Row<..>`: a type-level list of
//! `(key, value)` cells, where a column's key is its `expr::ColumnKey` and a
//! computed expression's is whatever identity it carries (`expr::Count`,
//! `window::RowNumber`, or an `AliasKey` supplied by `label!{}`). Reading a
//! field goes through `GetField`, the same indexed-lookup shape
//! `scope::Find` uses for tables.
//!
//! Keying by column rather than position is what makes adding a column to a
//! selection a non-breaking change, and what makes two same-typed columns
//! (`orders::user_id` and `orders::total` are both `BigInt`) impossible to
//! transpose. `into_tuple`/`into_tuples` recover the positional view where
//! destructuring is what's wanted.
//!
//! Naming a row type takes a type alias, and one long enough to trip
//! `clippy::type_complexity` — the same lint `qbrs-core` allows crate-wide.
//! Inference covers every use that doesn't cross a function boundary.
//!
//! **Known limitations**: a key selected twice makes `.get()` ambiguous
//! (`error[E0283]`, "multiple `impl`s satisfying ... `GetField`") rather
//! than silently resolving to the first — give one of them a `label!{}`
//! alias. `into_tuple` is implemented up to 16 columns; `Row` itself has no
//! such limit.

use std::marker::PhantomData;

use crate::expr::{ColumnKey, SqlType};
use crate::scope::{Here, There};

/// The empty row.
pub struct RowNil;

/// One field: `V`, filed under key `K`, followed by the rest in `Tail`.
pub struct RowCons<K, V, Tail> {
    value: V,
    tail: Tail,
    _key: PhantomData<fn() -> K>,
}

impl<K, V, Tail> RowCons<K, V, Tail> {
    #[doc(hidden)]
    pub fn new(value: V, tail: Tail) -> Self {
        RowCons {
            value,
            tail,
            _key: PhantomData,
        }
    }
}

/// Proof that a row holds a field under key `K`, found at compile-time-
/// inferred position `Idx`. `Idx` is never spelled out by callers, exactly
/// as in `scope::Find`, and is what keeps the two impls below structurally
/// distinct rather than overlapping.
#[diagnostic::on_unimplemented(
    message = "`{K}` is not in this query's selection",
    label = "a row can only be read by a key the query selected",
    note = "add `{K}` to the query's selection list to read it here"
)]
pub trait GetField<K, Idx> {
    type Value;
    fn get_field(&self) -> &Self::Value;
    fn into_field(self) -> Self::Value;
}

impl<K, V, Tail> GetField<K, Here> for RowCons<K, V, Tail> {
    type Value = V;
    fn get_field(&self) -> &V {
        &self.value
    }
    fn into_field(self) -> V {
        self.value
    }
}

impl<K, Other, V, Tail, I> GetField<K, There<I>> for RowCons<Other, V, Tail>
where
    Tail: GetField<K, I>,
{
    type Value = <Tail as GetField<K, I>>::Value;
    fn get_field(&self) -> &Self::Value {
        self.tail.get_field()
    }
    fn into_field(self) -> Self::Value {
        self.tail.into_field()
    }
}

impl<K, V: std::fmt::Debug, Tail: std::fmt::Debug> std::fmt::Debug for RowCons<K, V, Tail> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}, {:?}", self.value, self.tail)
    }
}

impl std::fmt::Debug for RowNil {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}

/// A decoded row. Its fields are fixed by the query's selection list, and
/// each is read by the same value that selected it.
pub struct Row<L>(L);

/// Values in selection order, without their keys: a key is a type, and the
/// two traits that give one a name (`expr::ColumnKey` and `AliasKey`) don't
/// cover the expression keys, so there is no name available for every field.
impl<L: std::fmt::Debug> std::fmt::Debug for Row<L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Row({:?})", self.0)
    }
}

impl<L> Row<L> {
    #[doc(hidden)]
    pub fn new(fields: L) -> Self {
        Row(fields)
    }

    /// `row.get(users::email)` — the key is the same value that appeared in
    /// the selection list, so there is no name to keep in sync and no
    /// position to get wrong.
    pub fn get<K: RowKey, Idx>(&self, _key: K) -> &<L as GetField<K::Key, Idx>>::Value
    where
        L: GetField<K::Key, Idx>,
    {
        self.0.get_field()
    }

    /// `get` by value, for when the field is wanted owned.
    pub fn into_get<K: RowKey, Idx>(self, _key: K) -> <L as GetField<K::Key, Idx>>::Value
    where
        L: GetField<K::Key, Idx>,
    {
        self.0.into_field()
    }

    /// The positional view: the plain tuple this selection would decode to
    /// if rows didn't exist. Also reachable as `row.into()`.
    pub fn into_tuple(self) -> L::Values
    where
        L: RowValues,
    {
        self.0.into_values()
    }
}

/// A row's fields as a plain tuple, in selection order.
pub trait RowValues {
    type Values;
    fn into_values(self) -> Self::Values;
}

/// Adds one element to the front of a tuple. The only place `Row`'s
/// positional view has an arity limit.
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

macro_rules! row_chain {
    ($k:ident $v:ident) => { RowCons<$k, $v, RowNil> };
    ($k:ident $v:ident, $($rest:tt)*) => { RowCons<$k, $v, row_chain!($($rest)*)> };
}

// `From<Row<..>> for (..)`, so a single row converts with plain `.into()`.
// One impl per arity: the trait's `Self` type has to be a concrete tuple,
// which rules out a single blanket impl over `RowValues::Values`.
macro_rules! row_into_tuple {
    ($($k:ident $v:ident),+) => {
        impl<$($k,)+ $($v,)+> From<Row<row_chain!($($k $v),+)>> for ($($v,)+) {
            fn from(row: Row<row_chain!($($k $v),+)>) -> Self {
                row.into_tuple()
            }
        }
    };
}
row_into_tuple!(K1 V1);
row_into_tuple!(K1 V1, K2 V2);
row_into_tuple!(K1 V1, K2 V2, K3 V3);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7, K8 V8);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7, K8 V8, K9 V9);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7, K8 V8, K9 V9, K10 V10);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7, K8 V8, K9 V9, K10 V10, K11 V11);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7, K8 V8, K9 V9, K10 V10, K11 V11, K12 V12);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7, K8 V8, K9 V9, K10 V10, K11 V11, K12 V12, K13 V13);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7, K8 V8, K9 V9, K10 V10, K11 V11, K12 V12, K13 V13, K14 V14);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7, K8 V8, K9 V9, K10 V10, K11 V11, K12 V12, K13 V13, K14 V14, K15 V15);
row_into_tuple!(K1 V1, K2 V2, K3 V3, K4 V4, K5 V5, K6 V6, K7 V7, K8 V8, K9 V9, K10 V10, K11 V11, K12 V12, K13 V13, K14 V14, K15 V15, K16 V16);

/// `Vec<Row<..>> -> Vec<(..)>`. A trait rather than `From`, which the orphan
/// rule forbids for `Vec` on both sides.
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

/// Maps a value written in a selection list to the type its field is filed
/// under. A `Column<C>` files under `C` itself, so the row's key list reads
/// as the column markers the schema declared rather than as wrappers.
pub trait RowKey: Copy + 'static {
    type Key: 'static;
}

impl<C: ColumnKey> RowKey for crate::expr::Column<C> {
    type Key = C;
}

/// The key of a selected expression that carries no identity of its own — a
/// `sql!{}` fragment, or any `Expr`. Deliberately not a `RowKey`, so it
/// cannot be named at a `.get()` call: such a field is reachable only
/// through `Row::into_tuple`, or by giving it a `label!{}` alias.
pub struct Anon;

/// A caller-declared output-column name, generated by `label!{}`. The name
/// reaches the SQL as the selected item's `AS`, and the type is what
/// `Row::get` looks the field up by.
pub trait AliasKey: RowKey<Key = Self> {
    const NAME: &'static str;
}

/// Declares an expression's own row key and the `row.<name>()` accessor
/// that reads it: `count()` and each window function get one, so a selection
/// that uses one of them needs nothing declared at the call site.
macro_rules! expr_key {
    ($key:ident, $accessor:ident, $method:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy)]
        pub struct $key;

        impl $crate::row::RowKey for $key {
            type Key = $key;
        }

        #[doc = $doc]
        pub trait $accessor<Idx> {
            type Value;
            fn $method(&self) -> &Self::Value;
        }

        impl<L, Idx> $accessor<Idx> for $crate::row::Row<L>
        where
            L: $crate::row::GetField<$key, Idx>,
        {
            type Value = <L as $crate::row::GetField<$key, Idx>>::Value;
            fn $method(&self) -> &Self::Value {
                self.get($key)
            }
        }
    };
}
pub(crate) use expr_key;

/// A selected item filed under an `AliasKey` instead of its own identity.
pub struct Aliased<K, Inner> {
    pub(crate) inner: Inner,
    _key: PhantomData<fn() -> K>,
}

impl<K, Inner> Aliased<K, Inner> {
    pub(crate) fn new(inner: Inner) -> Self {
        Aliased {
            inner,
            _key: PhantomData,
        }
    }
}

impl<C: ColumnKey> crate::expr::Column<C> {
    /// Files this column in the row under `key` rather than under itself —
    /// the way out of two tables' same-named columns colliding.
    pub fn alias<K: AliasKey>(self, _key: K) -> Aliased<K, Self> {
        Aliased::new(self)
    }
}

impl<Req, S: SqlType> crate::expr::Expr<Req, S> {
    pub fn alias<K: AliasKey>(self, _key: K) -> Aliased<K, Self> {
        Aliased::new(self)
    }
}

impl<K0, Req, S: SqlType> crate::expr::Keyed<K0, Req, S> {
    /// Replaces the identity this expression carries, so the same function
    /// can be selected more than once in one query.
    pub fn alias<K: AliasKey>(self, _key: K) -> Aliased<K, Self> {
        Aliased::new(self)
    }
}
