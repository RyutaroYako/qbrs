//! SQL types, columns, and the typed expression AST.
//!
//! `Expr<Req, S>` carries two purely phantom compile-time tags: `Req` (the
//! flat cons-list of tables this expression touches — see `scope::Superset`)
//! and `S` (its SQL type). The actual payload, `ExprKind`, is a plain closed
//! enum with no generics at all, so the renderer is never re-monomorphized
//! per query shape.

use std::marker::PhantomData;

use crate::render::Fragment;
use crate::scope::{Concat, Cons, MaybeNull, Nil, Table, WrapNullable};

/// A SQL scalar type. Implemented only by the closed set of leaf types
/// declared via `sql_leaf_type!` below, plus `Nullable<T>`.
pub trait SqlType: 'static {
    type Native;
}

/// The non-generic expression payload, constructible only inside this
/// crate: the typed `Expr<Req, S>` wrapper is the one way to build one,
/// which is what makes its `Req`/`S` tags mean anything.
#[derive(Debug, Clone)]
pub(crate) enum ExprKind {
    Column {
        table: &'static str,
        name: &'static str,
    },
    Value(Value),
    BinOp {
        op: BinOp,
        lhs: Box<ExprKind>,
        rhs: Box<ExprKind>,
    },
    And(Box<ExprKind>, Box<ExprKind>),
    Or(Box<ExprKind>, Box<ExprKind>),
    /// A condition with no operands left to check: what an empty `any_of`,
    /// `all_of` or `is_in` means.
    Always(bool),
    Not(Box<ExprKind>),
    /// `x IS NULL` / `x IS NOT NULL`. A separate node because `x = NULL` is
    /// never true in SQL, so equality can't stand in for it.
    IsNull {
        expr: Box<ExprKind>,
        negated: bool,
    },
    /// `x IN (a, b, ..)`. An empty list renders `FALSE`, which is what
    /// `IN ()` means and what SQL itself won't parse.
    InList {
        expr: Box<ExprKind>,
        values: Vec<ExprKind>,
    },
    /// An embedded piece of SQL: the `sql!{}` escape hatch, or a subquery
    /// rendered by `select::Select::fragment`.
    Raw(Fragment),
    /// `CAST(expr AS type)`. The target is a kind rather than a string
    /// because each dialect spells the type differently and the dialect
    /// isn't known until the query renders.
    Cast {
        expr: Box<ExprKind>,
        target: CastTarget,
    },
    /// `name(arg)`, or `name(*)` where there is no argument — the
    /// aggregates. A real node rather than a raw fragment because an
    /// argument is an expression the renderer has to recurse into, and
    /// because that is what lets its column count toward the expression's
    /// `Req`.
    Func {
        name: &'static str,
        arg: Option<Box<ExprKind>>,
    },
    /// `func OVER (PARTITION BY .. ORDER BY ..)`. `func` is rendered
    /// literally: it's always one of the closed set of niladic ranking
    /// functions, so there's no sub-expression to recurse into.
    /// `partition_by`/`order_by` *are* full `ExprKind`s, since they can
    /// reference real columns.
    Window {
        func: &'static str,
        partition_by: Vec<ExprKind>,
        order_by: Vec<(ExprKind, SortDir)>,
    },
}

/// Sort direction. Shared by `ORDER BY` (`select::OrderKey`) and window
/// functions' `OVER (.. ORDER BY ..)` (`window::Window`), hence living here
/// rather than in either module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

/// The types an aggregate is cast back into, so its result stays inside the
/// closed set of types this crate has natives for.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastTarget {
    BigInt,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    Like,
}

/// A closed, non-generic sum of every literal value the crate can bind as a
/// query parameter. Deliberately not `Box<dyn ToSql>`: a plain enum keeps
/// the renderer free of vtable dispatch and lets it stay a single
/// non-generic function no matter how many query shapes exist.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    I32(i32),
    I64(i64),
    F64(f64),
    Text(String),
    Bool(bool),
    Bytes(Vec<u8>),
    /// A NULL of a specific base type, *not* a single untyped `Null`: a
    /// Postgres bind declares a parameter type even when the value is NULL,
    /// and a mismatched one can fail query planning. Each `NullX` variant
    /// lets the execution layer bind `Option::<X>::None` with the right
    /// type.
    NullI32,
    NullI64,
    NullF64,
    NullText,
    NullBool,
    NullBytes,
    #[cfg(feature = "chrono")]
    Timestamptz(chrono::DateTime<chrono::Utc>),
    #[cfg(feature = "chrono")]
    NullTimestamptz,
    #[cfg(feature = "chrono")]
    Date(chrono::NaiveDate),
    #[cfg(feature = "chrono")]
    NullDate,
    #[cfg(feature = "uuid")]
    Uuid(uuid::Uuid),
    #[cfg(feature = "uuid")]
    NullUuid,
    #[cfg(feature = "decimal")]
    Numeric(rust_decimal::Decimal),
    #[cfg(feature = "decimal")]
    NullNumeric,
    /// A named placeholder in a `prepare!{}`-built query, not yet resolved
    /// to a concrete value. It rides the existing `Vec<Value>` parameter
    /// pipeline: rendering doesn't care what's *inside* a `Value`, only that
    /// there's one per placeholder position. `select::Prepared` substitutes
    /// real values before binding.
    Placeholder(&'static str),
}

macro_rules! value_from {
    ($ty:ty, $variant:ident) => {
        impl From<$ty> for Value {
            fn from(v: $ty) -> Self {
                Value::$variant(v)
            }
        }
    };
}
value_from!(i32, I32);
value_from!(i64, I64);
value_from!(f64, F64);
value_from!(String, Text);
value_from!(bool, Bool);
value_from!(Vec<u8>, Bytes);
#[cfg(feature = "chrono")]
value_from!(chrono::DateTime<chrono::Utc>, Timestamptz);
#[cfg(feature = "chrono")]
value_from!(chrono::NaiveDate, Date);
#[cfg(feature = "uuid")]
value_from!(uuid::Uuid, Uuid);
#[cfg(feature = "decimal")]
value_from!(rust_decimal::Decimal, Numeric);

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}

/// A typed SQL expression. `Req` is the (possibly empty) flat list of
/// tables this expression references — see the module docs and
/// `scope::Superset` for how that's checked against a query's actual scope
/// at the point the expression is used, not at the point it's built. This
/// is what lets `orders::user_id.eq(users::id)` be a plain, portable value
/// with no dependency on which query it'll eventually be used in.
pub struct Expr<Req, S: SqlType> {
    pub(crate) kind: ExprKind,
    _marker: PhantomData<fn() -> (Req, S)>,
}

impl<Req, S: SqlType> Expr<Req, S> {
    pub(crate) fn from_kind(kind: ExprKind) -> Self {
        Expr {
            kind,
            _marker: PhantomData,
        }
    }
}

// Manual Clone: `#[derive(Clone)]` would incorrectly require `Req: Clone`
// and `S: Clone`, even though both are purely phantom tags.
impl<Req, S: SqlType> Clone for Expr<Req, S> {
    fn clone(&self) -> Self {
        Expr::from_kind(self.kind.clone())
    }
}

/// Converts a value into a typed expression, tagging it with the set of
/// tables it references (`Nil` for a plain literal, `Cons<T, Nil>` for a
/// bare column, or whatever `Req` an already-built `Expr` carries).
#[diagnostic::on_unimplemented(
    message = "`{Self}` can't be used as a SQL expression of type `{S}`",
    label = "a column, a literal, an aggregate, or a `sql!{{}}` fragment can be; a `label!` name is not one, and neither is an `Option` — `= NULL` is never true in SQL, so the question is `.is_null()`"
)]
pub trait IntoExpr<S: SqlType> {
    type Req;
    fn into_expr(self) -> Expr<Self::Req, S>;
}

impl<Req, S: SqlType> IntoExpr<S> for Expr<Req, S> {
    type Req = Req;
    fn into_expr(self) -> Expr<Req, S> {
        self
    }
}

/// A column's compile-time identity. One implementor per column in the
/// schema, generated by `#[derive(Table)]` (and by `with!{}` for a CTE's
/// pseudo-columns), which is what lets a column be a *key* — two columns of
/// the same table and SQL type are still distinct types here, so a row can
/// be indexed by column without ambiguity.
pub trait ColumnKey: crate::row::Named + Copy + 'static {
    type Table: Table;
    type Sql: SqlType;
}

/// A column reference, identified entirely by its `ColumnKey`. Generated
/// per-field by `#[derive(Table)]` as a `pub const NAME: Column<..>` inside
/// each table's module (e.g. `users::id`). A plain, `Copy` value — not tied
/// to any particular query — which is what lets it be reused across queries
/// and passed as an ordinary function argument instead of through a
/// scope-bound cursor closure.
pub struct Column<C: ColumnKey>(PhantomData<C>);

impl<C: ColumnKey> Column<C> {
    pub const fn new() -> Self {
        Column(PhantomData)
    }
}

impl<C: ColumnKey> Default for Column<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: ColumnKey> Clone for Column<C> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<C: ColumnKey> Copy for Column<C> {}

impl<C: ColumnKey> IntoExpr<C::Sql> for Column<C> {
    type Req = Cons<C::Table, Nil>;
    fn into_expr(self) -> Expr<Self::Req, C::Sql> {
        Expr::from_kind(ExprKind::Column {
            table: <C::Table as Table>::NAME,
            name: <C as crate::row::Named>::NAME,
        })
    }
}

/// An expression that carries its own row key `K`: `count()` and the window
/// functions, whose identity is the function that produced them. Selecting
/// two of the same one into a row is what `row::Row::get` rejects, and what
/// `Expr::alias` exists to resolve.
pub struct Keyed<K, Req, S: SqlType> {
    pub(crate) kind: ExprKind,
    _marker: PhantomData<fn() -> (K, Req, S)>,
}

impl<K, Req, S: SqlType> Keyed<K, Req, S> {
    pub(crate) fn from_kind(kind: ExprKind) -> Self {
        Keyed {
            kind,
            _marker: PhantomData,
        }
    }
}

impl<K, Req, S: SqlType> Clone for Keyed<K, Req, S> {
    fn clone(&self) -> Self {
        Keyed::from_kind(self.kind.clone())
    }
}

impl<K, Req, S: SqlType> IntoExpr<S> for Keyed<K, Req, S> {
    type Req = Req;
    fn into_expr(self) -> Expr<Req, S> {
        Expr::from_kind(self.kind)
    }
}

/// Which SQL types may be compared with each other. A nullable column and a
/// non-nullable one hold the same values, so `users::manager_id.eq(users::id)`
/// is an ordinary join predicate; without this relation the two would be
/// unrelated types and every optional foreign key would be unwritable.
///
/// `Option` still isn't an expression, so `.eq(None)` remains unwritable:
/// `x = NULL` is never true, and `.is_null()` is the question that was meant.
#[diagnostic::on_unimplemented(
    message = "`{Self}` and `{Other}` aren't comparable",
    label = "both sides of a comparison must be the same SQL type, or two numeric ones",
    note = "nullability doesn't matter here: a `Nullable<T>` compares with a `T`"
)]
pub trait Comparable<Other: SqlType>: SqlType {}

impl<T: SqlType> Comparable<T> for T {}
impl<T: SqlType> Comparable<crate::scope::Nullable<T>> for T {}
impl<T: SqlType> Comparable<T> for crate::scope::Nullable<T> {}

/// Numeric widths compare across each other, so an untyped literal doesn't
/// have to be annotated to match the column it's tested against.
macro_rules! comparable_across {
    ($($a:ty => $b:ty),+ $(,)?) => {
        $(
            impl Comparable<$b> for $a {}
            impl Comparable<$b> for crate::scope::Nullable<$a> {}
            impl Comparable<crate::scope::Nullable<$b>> for $a {}
            impl Comparable<crate::scope::Nullable<$b>> for crate::scope::Nullable<$a> {}
        )+
    };
}
comparable_across!(
    Integer => BigInt,
    BigInt => Integer,
    Integer => Real,
    Real => Integer,
    BigInt => Real,
    Real => BigInt,
);

/// A caller-declared output-column name, generated by `label!{}`. Being a
/// marker over `Named` is what keeps `label!{}` the only way to make one,
/// and `Named::NAME` the only place the name is written.
pub trait AliasKey: crate::row::Named + Copy + 'static {}

/// A table-referencing expression together with the SQL type its author
/// says it decodes to. A computed expression's NULL-ability doesn't follow
/// from any one column's join — `coalesce(o.total, 0)` isn't nullable and
/// `o.total + 1` is — so it is the one thing the builder can't derive, and
/// the call site is the only honest place to state it.
pub struct Declared<Req, S: SqlType> {
    pub(crate) kind: ExprKind,
    _marker: PhantomData<fn() -> (Req, S)>,
}

impl<Req, S: SqlType> Clone for Declared<Req, S> {
    fn clone(&self) -> Self {
        Declared {
            kind: self.kind.clone(),
            _marker: PhantomData,
        }
    }
}

impl<Req, S: SqlType> Expr<Req, S> {
    /// States what this expression decodes to, which is what makes it
    /// selectable: `S` was inferred from whatever built the expression, and
    /// an inference can contradict the join the query actually has.
    pub fn decodes_as<S2: SqlType>(self) -> Declared<Req, S2> {
        Declared {
            kind: self.kind,
            _marker: PhantomData,
        }
    }
}

/// A selected item filed under an `AliasKey` instead of under its own
/// identity, and rendered with that name as its `AS`.
pub struct Aliased<K, Inner> {
    pub(crate) inner: Inner,
    _key: PhantomData<fn() -> K>,
}

impl<K, Inner: Clone> Clone for Aliased<K, Inner> {
    fn clone(&self) -> Self {
        Aliased::new(self.inner.clone())
    }
}

impl<K, Inner> Aliased<K, Inner> {
    pub(crate) fn new(inner: Inner) -> Self {
        Aliased {
            inner,
            _key: PhantomData,
        }
    }
}

/// Files a selected item under a declared name: the way to select the same
/// expression twice, and the way out of two tables' same-named columns
/// colliding.
pub trait AliasExt: Sized {
    fn alias<K: AliasKey>(self, _key: K) -> Aliased<K, Self> {
        Aliased::new(self)
    }
}
impl<C: ColumnKey> AliasExt for Column<C> {}
impl<Req, S: SqlType> AliasExt for Expr<Req, S> {}
impl<K, Req, S: SqlType> AliasExt for Keyed<K, Req, S> {}
impl<Req, S: SqlType> AliasExt for Declared<Req, S> {}

/// Comparison/boolean-combinator methods, blanket-implemented for anything
/// convertible to a typed expression (columns, literals, and `Expr` itself).
/// Kept separate from `IntoExpr` so one blanket impl can serve all three.
// Every combinator here consumes `self`, this crate's builders being
// by-value throughout; `is_null`/`is_in` are combinators, not predicates on
// an existing value.
#[allow(clippy::wrong_self_convention)]
pub trait ExprMethods<S: SqlType>: IntoExpr<S> + Sized {
    fn eq<Rhs, S2: SqlType>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S2>,
        S: Comparable<S2>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Eq, self, rhs)
    }

    fn ne<Rhs, S2: SqlType>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S2>,
        S: Comparable<S2>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Ne, self, rhs)
    }

    fn lt<Rhs, S2: SqlType>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S2>,
        S: Comparable<S2>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Lt, self, rhs)
    }

    fn lte<Rhs, S2: SqlType>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S2>,
        S: Comparable<S2>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Lte, self, rhs)
    }

    fn gt<Rhs, S2: SqlType>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S2>,
        S: Comparable<S2>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Gt, self, rhs)
    }

    fn gte<Rhs, S2: SqlType>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S2>,
        S: Comparable<S2>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Gte, self, rhs)
    }

    /// `x IS NULL`. Not expressible as `.eq(..)`: comparing to NULL with `=`
    /// yields NULL, never true, so the two are different questions.
    fn is_null(self) -> Expr<Self::Req, Bool> {
        Expr::from_kind(ExprKind::IsNull {
            expr: Box::new(self.into_expr().kind),
            negated: false,
        })
    }

    fn is_not_null(self) -> Expr<Self::Req, Bool> {
        Expr::from_kind(ExprKind::IsNull {
            expr: Box::new(self.into_expr().kind),
            negated: true,
        })
    }

    /// `x IN (a, b, ..)` over a runtime-length list of literals, each bound
    /// as its own parameter. An empty list renders `FALSE`.
    ///
    /// **Known limitation**: the list holds values, not expressions — a
    /// column reference on the right needs the table it belongs to folded
    /// into `Req`, which is the same design `sql!{}` covers today.
    fn is_in<I, S2: SqlType>(self, values: I) -> Expr<Self::Req, Bool>
    where
        I: IntoIterator,
        I::Item: IntoExpr<S2, Req = Nil>,
        S: Comparable<S2>,
    {
        Expr::from_kind(ExprKind::InList {
            expr: Box::new(self.into_expr().kind),
            values: values.into_iter().map(|v| v.into_expr().kind).collect(),
        })
    }
}

/// True when any of the conditions is. Takes a runtime-length collection,
/// the way `is_in` takes a runtime-length list of values, so the `OR` a
/// search box needs doesn't have to be folded by hand — folding one by one
/// grows `Req` and stops type-checking after the first pair. An empty
/// collection matches nothing, which is what `is_in([])` says too.
pub fn any_of<Req>(conds: impl IntoIterator<Item = Expr<Req, Bool>>) -> Expr<Req, Bool> {
    combine(conds, false)
}

/// True when all of them are. An empty collection matches everything, which
/// is what a `WHERE` with no conditions does.
pub fn all_of<Req>(conds: impl IntoIterator<Item = Expr<Req, Bool>>) -> Expr<Req, Bool> {
    combine(conds, true)
}

fn combine<Req>(conds: impl IntoIterator<Item = Expr<Req, Bool>>, all: bool) -> Expr<Req, Bool> {
    let mut folded: Option<ExprKind> = None;
    for cond in conds {
        folded = Some(match folded {
            None => cond.kind,
            Some(acc) if all => ExprKind::And(Box::new(acc), Box::new(cond.kind)),
            Some(acc) => ExprKind::Or(Box::new(acc), Box::new(cond.kind)),
        });
    }
    Expr::from_kind(folded.unwrap_or(ExprKind::Always(all)))
}

impl<S: SqlType, T: IntoExpr<S>> ExprMethods<S> for T {}

fn bin_op<Lhs, Rhs, S: SqlType, S2: SqlType>(
    op: BinOp,
    lhs: Lhs,
    rhs: Rhs,
) -> Expr<<Lhs::Req as Concat<Rhs::Req>>::Output, Bool>
where
    Lhs: IntoExpr<S>,
    Rhs: IntoExpr<S2>,
    Lhs::Req: Concat<Rhs::Req>,
{
    Expr::from_kind(ExprKind::BinOp {
        op,
        lhs: Box::new(lhs.into_expr().kind),
        rhs: Box::new(rhs.into_expr().kind),
    })
}

/// `.like()` is text-only, so it's a separate trait rather than part of the
/// generic `ExprMethods` — still blanket-implemented, so it works directly
/// on a text column just like `.eq()` does.
#[diagnostic::on_unimplemented(
    message = "`LIKE` needs a text expression, and `{Self}` isn't one",
    label = "only `Text` and `Nullable<Text>` columns and expressions accept `.like(..)`"
)]
pub trait TextExprMethods<S: Comparable<Text>>: IntoExpr<S> + Sized {
    fn like<Rhs, S2: Comparable<Text>>(
        self,
        rhs: Rhs,
    ) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S2>,
        S: Comparable<S2>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Like, self, rhs)
    }
}
impl<S: Comparable<Text>, T: IntoExpr<S>> TextExprMethods<S> for T {}

impl<Req> Expr<Req, Bool> {
    pub fn and<Req2>(self, rhs: Expr<Req2, Bool>) -> Expr<<Req as Concat<Req2>>::Output, Bool>
    where
        Req: Concat<Req2>,
    {
        Expr::from_kind(ExprKind::And(Box::new(self.kind), Box::new(rhs.kind)))
    }

    pub fn or<Req2>(self, rhs: Expr<Req2, Bool>) -> Expr<<Req as Concat<Req2>>::Output, Bool>
    where
        Req: Concat<Req2>,
    {
        Expr::from_kind(ExprKind::Or(Box::new(self.kind), Box::new(rhs.kind)))
    }
}

/// `!condition`, not `condition.not()`: the standard `Not` trait reads more
/// naturally at call sites than a same-named inherent method.
impl<Req> std::ops::Not for Expr<Req, Bool> {
    type Output = Expr<Req, Bool>;
    fn not(self) -> Self::Output {
        Expr::from_kind(ExprKind::Not(Box::new(self.kind)))
    }
}

/// Declares a leaf (base) SQL type: the marker struct, its `SqlType` impl,
/// its `WrapNullable<MaybeNull>` impl, and `IntoExpr` from its native Rust
/// type. One concrete, non-generic impl per type — a blanket
/// `impl<T: SqlType> WrapNullable<MaybeNull> for T` would conflict with
/// `Nullable<T>`'s own impl (see `scope::WrapNullable`).
/// A base SQL type's typed NULL — see `Value::NullI32` etc. for why this
/// can't just be a single untyped `Value::Null`.
pub trait NullValue: SqlType {
    const NULL_VALUE: Value;
}

macro_rules! sql_leaf_type {
    ($name:ident, $native:ty, $null_variant:ident) => {
        pub struct $name;

        impl SqlType for $name {
            type Native = $native;
        }

        impl crate::scope::WrapNullable<MaybeNull> for $name {
            type Output = crate::scope::Nullable<$name>;
        }

        impl IntoExpr<$name> for $native {
            type Req = Nil;
            fn into_expr(self) -> Expr<Nil, $name> {
                Expr::from_kind(ExprKind::Value(Value::from(self)))
            }
        }

        impl NullValue for $name {
            const NULL_VALUE: Value = Value::$null_variant;
        }

        impl crate::row::SameShape<$native> for $native {}
        impl crate::row::SameShape<::std::option::Option<$native>>
            for ::std::option::Option<$native>
        {
        }

        // A nullable `prepare!{}` parameter binds through here, which is what
        // the typed `NullX` variants exist for.
        impl From<::std::option::Option<$native>> for Value {
            fn from(v: ::std::option::Option<$native>) -> Self {
                match v {
                    ::std::option::Option::Some(x) => Value::from(x),
                    ::std::option::Option::None => Value::$null_variant,
                }
            }
        }
    };
}

sql_leaf_type!(Integer, i32, NullI32);
sql_leaf_type!(BigInt, i64, NullI64);
sql_leaf_type!(Real, f64, NullF64);
sql_leaf_type!(Text, String, NullText);
sql_leaf_type!(Bool, bool, NullBool);
sql_leaf_type!(Bytes, Vec<u8>, NullBytes);

// Types a database has and Rust doesn't: each decodes to the crate its
// feature names, so a schema that has no `timestamptz` column pays for none
// of it.
#[cfg(feature = "chrono")]
sql_leaf_type!(Timestamptz, chrono::DateTime<chrono::Utc>, NullTimestamptz);
#[cfg(feature = "chrono")]
sql_leaf_type!(Date, chrono::NaiveDate, NullDate);
#[cfg(feature = "uuid")]
sql_leaf_type!(Uuid, uuid::Uuid, NullUuid);
#[cfg(feature = "decimal")]
sql_leaf_type!(Numeric, rust_decimal::Decimal, NullNumeric);

// Ergonomic extra: allow `&str` literals directly, without forcing
// `.to_string()` at every call site.
impl IntoExpr<Text> for &String {
    type Req = Nil;
    fn into_expr(self) -> Expr<Nil, Text> {
        Expr::from_kind(ExprKind::Value(Value::Text(self.clone())))
    }
}

impl IntoExpr<Text> for &str {
    type Req = Nil;
    fn into_expr(self) -> Expr<Nil, Text> {
        Expr::from_kind(ExprKind::Value(Value::Text(self.to_string())))
    }
}

impl<S: SqlType> SqlType for crate::scope::Nullable<S> {
    type Native = Option<S::Native>;
}

crate::row::expr_key!(
    Count,
    HasCount,
    count,
    "The identity a selected `count(*)` is filed under in a row.",
    'c',
    'o',
    'u',
    'n',
    't'
);

/// `count(*)` as a rendered selection item, for `Select::count_sql`.
pub(crate) fn count_item() -> crate::render::SelectItem {
    crate::render::SelectItem::bare(count_star())
}

fn count_star() -> ExprKind {
    ExprKind::Func {
        name: "count",
        arg: None,
    }
}

/// Counts rows. `count_of(column)` counts that column's non-NULL values,
/// which is the different question a `LEFT JOIN` makes visible.
pub fn count() -> Keyed<Count, Nil, BigInt> {
    Keyed::from_kind(count_star())
}

/// What `sum(..)` of a column decodes to. `sum` is NULL over zero rows, so
/// every result is nullable however the column was declared.
/// `CAST` keeps the widened type a database picks for a sum inside the
/// closed set of types this crate has: Postgres returns `numeric` for
/// `sum(bigint)` and `avg(int)`, neither of which has a native here.
pub trait Summable: SqlType {
    type Sum: SqlType;
    const SUM_CAST: Option<CastTarget>;
    const AVG_CAST: Option<CastTarget> = Some(CastTarget::Double);
}
impl Summable for Integer {
    type Sum = crate::scope::Nullable<BigInt>;
    const SUM_CAST: Option<CastTarget> = None;
}
impl Summable for BigInt {
    type Sum = crate::scope::Nullable<BigInt>;
    const SUM_CAST: Option<CastTarget> = Some(CastTarget::BigInt);
}
impl Summable for Real {
    type Sum = crate::scope::Nullable<Real>;
    const SUM_CAST: Option<CastTarget> = None;
    const AVG_CAST: Option<CastTarget> = None;
}
impl<T: Summable> Summable for crate::scope::Nullable<T> {
    type Sum = T::Sum;
    const SUM_CAST: Option<CastTarget> = T::SUM_CAST;
    const AVG_CAST: Option<CastTarget> = T::AVG_CAST;
}

/// The row key an aggregate over `C` is filed under: distinct per function
/// *and* per column, so `sum(a)` and `sum(b)` don't collide, and named after
/// the column so a DTO field or a CTE column can match it.
pub struct Agg<Op, C>(PhantomData<fn() -> (Op, C)>);

impl<Op, C> Clone for Agg<Op, C> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<Op, C> Copy for Agg<Op, C> {}

#[doc(hidden)]
impl<Op: 'static, C: crate::row::Named + 'static> crate::row::Named for Agg<Op, C> {
    type Name = <C as crate::row::Named>::Name;
    const NAME: &'static str = <C as crate::row::Named>::NAME;
}

fn aggregate_kind<C: ColumnKey>(name: &'static str, cast: Option<CastTarget>) -> ExprKind {
    let call = ExprKind::Func {
        name,
        arg: Some(Box::new(ExprKind::Column {
            table: <C::Table as Table>::NAME,
            name: <C as crate::row::Named>::NAME,
        })),
    };
    match cast {
        Some(target) => ExprKind::Cast {
            expr: Box::new(call),
            target,
        },
        None => call,
    }
}

macro_rules! aggregate {
    ($op:ident, $func:ident, $sql:literal, $out:ty, $bound:path, $cast:expr, $doc:literal) => {
        #[doc = $doc]
        pub struct $op;

        #[doc = $doc]
        pub fn $func<C: ColumnKey>(
            _column: Column<C>,
        ) -> Keyed<Agg<$op, C>, Cons<C::Table, Nil>, $out>
        where
            C::Sql: $bound,
            $out: SqlType,
        {
            Keyed::from_kind(aggregate_kind::<C>($sql, $cast))
        }
    };
}

aggregate!(
    Sum,
    sum,
    "sum",
    <C::Sql as Summable>::Sum,
    Summable,
    <C::Sql as Summable>::SUM_CAST,
    "`sum(column)`. NULL over zero rows, so the result is always nullable."
);
aggregate!(
    Min,
    min,
    "min",
    <C::Sql as WrapNullable<MaybeNull>>::Output,
    WrapNullable<MaybeNull>,
    None,
    "`min(column)`. NULL over zero rows."
);
aggregate!(
    Max,
    max,
    "max",
    <C::Sql as WrapNullable<MaybeNull>>::Output,
    WrapNullable<MaybeNull>,
    None,
    "`max(column)`. NULL over zero rows."
);
aggregate!(
    Avg,
    avg,
    "avg",
    crate::scope::Nullable<Real>,
    Summable,
    <C::Sql as Summable>::AVG_CAST,
    "`avg(column)`. NULL over zero rows."
);
aggregate!(
    CountOf,
    count_of,
    "count",
    BigInt,
    SqlType,
    None,
    "`count(column)` — non-NULL values, unlike `count()`'s `count(*)` rows."
);

/// Counts the `?` placeholders in a `sql!` text, so the macro can compare
/// that count with the number of values it was handed while both are still
/// constants. `??` is a literal `?` and counts for nothing.
#[doc(hidden)]
pub const fn placeholder_count(sql: &str) -> usize {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut count = 0;
    while i < bytes.len() {
        if bytes[i] == b'?' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'?' {
                i += 2;
                continue;
            }
            count += 1;
        }
        i += 1;
    }
    count
}

/// The one door into `ExprKind` from outside the crate, and the only shape
/// that needs one: `sql!{}` expands in the caller's. `Req` is pinned to
/// `Nil` rather than being a parameter, so a raw fragment can't claim a
/// scope it hasn't got. Reached through `sql!`, which is what checks that
/// every `?` has a value.
#[doc(hidden)]
pub fn raw_expr<S: SqlType>(sql: &'static str, params: Vec<Value>) -> Expr<Nil, S> {
    Expr::from_kind(ExprKind::Raw(Fragment::from_authored(sql, params)))
}

/// A named, typed placeholder: usable anywhere a value of type `S` is
/// expected (`.eq(placeholder::<Integer>("id"))`), rendered as a normal
/// bound parameter but resolved to a concrete value at
/// `Prepared::load()` time. `prepare!{}` is the intended entry point
/// rather than this function, since it also generates the typed `Params`
/// struct that makes a missing or misspelled placeholder a compile error.
#[doc(hidden)]
pub fn placeholder<S: SqlType>(name: &'static str) -> Expr<Nil, S> {
    Expr::from_kind(ExprKind::Value(Value::Placeholder(name)))
}
