//! SQL types, columns, and the typed expression AST.
//!
//! `Expr<Req, S>` carries two purely phantom compile-time tags: `Req` (the
//! flat cons-list of tables this expression touches — see `scope::Superset`)
//! and `S` (its SQL type). The actual payload, `ExprKind`, is a plain closed
//! enum with no generics at all, so the renderer is never re-monomorphized
//! per query shape (unlike diesel's `QueryFragment`/`walk_ast`, which is
//! itself generic over the query's type).

use std::marker::PhantomData;

use crate::scope::{Concat, Cons, MaybeNull, Nil, Table};

/// A SQL scalar type. Implemented only by the closed set of leaf types
/// declared via `sql_leaf_type!` below, plus `Nullable<T>`.
pub trait SqlType: 'static {
    type Native;
}

/// The non-generic expression payload. Never constructed or matched on
/// outside this crate — the typed `Expr<Req, S>` wrapper is the only
/// supported way to build one, which is what keeps the `Req`/`S` tags
/// trustworthy (a hand-built `ExprKind` could otherwise claim to be
/// anything).
#[derive(Debug, Clone)]
pub enum ExprKind {
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
    And(Vec<ExprKind>),
    Or(Vec<ExprKind>),
    Not(Box<ExprKind>),
    /// Escape hatch for raw SQL fragments (the `sql!{}` macro target).
    /// Placeholders in `text` are positional (`?`, rendered per-dialect at
    /// render time); `params` are bound in order.
    Raw {
        text: String,
        params: Vec<Value>,
    },
    /// `func OVER (PARTITION BY .. ORDER BY ..)`. `func` is rendered
    /// literally (not recursively as an `ExprKind`) — see
    /// `window::WindowFunc`'s doc comment for why: it's always one of the
    /// closed set of niladic ranking functions (`row_number()`, `rank()`,
    /// `dense_rank()`) for now, so there is no sub-expression to recurse
    /// into yet. `partition_by`/`order_by` *are* full `ExprKind`s (they can
    /// reference real columns), which is why this variant carries `Vec`s of
    /// them rather than pre-rendered text.
    Window {
        func: String,
        partition_by: Vec<ExprKind>,
        order_by: Vec<(ExprKind, SortDir)>,
    },
}

/// Sort direction — shared by `ORDER BY` (`select::OrderKey`) and window
/// functions' `OVER (.. ORDER BY ..)` (`window::Window`), which is why it
/// lives here rather than in either of those modules specifically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
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
/// query parameter. Deliberately not `Box<dyn ToSql>` — keeping this a
/// plain enum means the SQL renderer has no vtable dispatch anywhere in its
/// hot path, and stays a single non-generic function regardless of how many
/// distinct `Req`/`S` combinations exist in the calling crate.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    I32(i32),
    I64(i64),
    F64(f64),
    Text(String),
    Bool(bool),
    Bytes(Vec<u8>),
    /// A NULL of a specific base type. Deliberately *not* a single untyped
    /// `Null` variant: a real Postgres bind still declares a parameter type
    /// (OID) even when the value is NULL, and a mismatched type there can
    /// fail query planning (`operator does not exist: text = integer`)
    /// even though the value itself is NULL. Each `NullX` variant lets the
    /// execution layer bind `Option::<X>::None` with the right type.
    NullI32,
    NullI64,
    NullF64,
    NullText,
    NullBool,
    NullBytes,
    /// A named placeholder in a `prepare!{}`-built query, not yet resolved
    /// to a concrete value. Reusing the existing `Vec<Value>` parameter
    /// pipeline for this (rather than introducing a parallel "parameter
    /// slot" type threaded through every render/bind call site) is what
    /// keeps named placeholders a small, additive feature: rendering
    /// doesn't care what's *inside* a `Value` it's binding, only that
    /// there's one per placeholder position — see `select::Prepared` for
    /// where these get substituted for real values before binding.
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
    pub kind: ExprKind,
    _marker: PhantomData<fn() -> (Req, S)>,
}

impl<Req, S: SqlType> Expr<Req, S> {
    #[doc(hidden)]
    pub fn from_kind(kind: ExprKind) -> Self {
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

/// A column reference: table `T`, SQL type `S`. Generated per-field by the
/// `#[derive(Table)]` macro as a `pub const NAME: Column<Table, SqlType>`
/// inside each table's module (e.g. `users::id`). A plain, `Copy` value —
/// not tied to any particular query — which is what lets it be reused
/// across queries and passed as an ordinary function argument instead of
/// through a scope-bound cursor closure.
pub struct Column<T: Table, S: SqlType> {
    pub name: &'static str,
    _marker: PhantomData<(T, S)>,
}

impl<T: Table, S: SqlType> Column<T, S> {
    pub const fn new(name: &'static str) -> Self {
        Column {
            name,
            _marker: PhantomData,
        }
    }
}

impl<T: Table, S: SqlType> Clone for Column<T, S> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Table, S: SqlType> Copy for Column<T, S> {}

impl<T: Table, S: SqlType> IntoExpr<S> for Column<T, S> {
    type Req = Cons<T, Nil>;
    fn into_expr(self) -> Expr<Self::Req, S> {
        Expr::from_kind(ExprKind::Column {
            table: T::NAME,
            name: self.name,
        })
    }
}

/// Comparison/boolean-combinator methods, blanket-implemented for anything
/// convertible to a typed expression (columns, literals, and `Expr` itself).
/// Splitting this out from `IntoExpr` is what lets a single blanket impl
/// provide `.eq()` etc. on all three without conflicting impls.
pub trait ExprMethods<S: SqlType>: IntoExpr<S> + Sized {
    fn eq<Rhs>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Eq, self, rhs)
    }

    fn ne<Rhs>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Ne, self, rhs)
    }

    fn lt<Rhs>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Lt, self, rhs)
    }

    fn lte<Rhs>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Lte, self, rhs)
    }

    fn gt<Rhs>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Gt, self, rhs)
    }

    fn gte<Rhs>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<S>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Gte, self, rhs)
    }
}

impl<S: SqlType, T: IntoExpr<S>> ExprMethods<S> for T {}

fn bin_op<Lhs, Rhs, S: SqlType>(
    op: BinOp,
    lhs: Lhs,
    rhs: Rhs,
) -> Expr<<Lhs::Req as Concat<Rhs::Req>>::Output, Bool>
where
    Lhs: IntoExpr<S>,
    Rhs: IntoExpr<S>,
    Lhs::Req: Concat<Rhs::Req>,
{
    Expr::from_kind(ExprKind::BinOp {
        op,
        lhs: Box::new(lhs.into_expr().kind),
        rhs: Box::new(rhs.into_expr().kind),
    })
}

/// `.like()` is Text-only (doesn't make sense for other SQL types), so it's
/// a separate trait rather than part of the generic `ExprMethods` — but
/// still blanket-implemented the same way, so it's usable directly on a
/// `Column<T, Text>` (not just on an already-converted `Expr<Req, Text>>`),
/// matching `.eq()`'s ergonomics.
pub trait TextExprMethods: IntoExpr<Text> + Sized {
    fn like<Rhs>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Rhs: IntoExpr<Text>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Like, self, rhs)
    }
}
impl<T: IntoExpr<Text>> TextExprMethods for T {}

impl<Req> Expr<Req, Bool> {
    pub fn and<Req2>(self, rhs: Expr<Req2, Bool>) -> Expr<<Req as Concat<Req2>>::Output, Bool>
    where
        Req: Concat<Req2>,
    {
        Expr::from_kind(ExprKind::And(vec![self.kind, rhs.kind]))
    }

    pub fn or<Req2>(self, rhs: Expr<Req2, Bool>) -> Expr<<Req as Concat<Req2>>::Output, Bool>
    where
        Req: Concat<Req2>,
    {
        Expr::from_kind(ExprKind::Or(vec![self.kind, rhs.kind]))
    }
}

/// `!condition`, not `condition.not()` — implementing the standard
/// `Not` trait instead of a same-named inherent method is what clippy's
/// `should_implement_trait` lint is steering toward, and it reads more
/// naturally at call sites besides.
impl<Req> std::ops::Not for Expr<Req, Bool> {
    type Output = Expr<Req, Bool>;
    fn not(self) -> Self::Output {
        Expr::from_kind(ExprKind::Not(Box::new(self.kind)))
    }
}

/// Declares a leaf (base) SQL type: the marker struct, its `SqlType` impl,
/// its `WrapNullable<MaybeNull>` impl, and `IntoExpr` from its native Rust
/// type. Deliberately generates one concrete, non-generic impl per type
/// rather than a single blanket `impl<T: SqlType> WrapNullable<MaybeNull>
/// for T` — the Phase 0 spike found that blanket form conflicts (E0119)
/// with `Nullable<T>`'s own `WrapNullable<MaybeNull>` impl, the same
/// coherence trap `Find`/`Contains` needed `Here`/`There<I>` to route
/// around.
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
    };
}

sql_leaf_type!(Integer, i32, NullI32);
sql_leaf_type!(BigInt, i64, NullI64);
sql_leaf_type!(Real, f64, NullF64);
sql_leaf_type!(Text, String, NullText);
sql_leaf_type!(Bool, bool, NullBool);
sql_leaf_type!(Bytes, Vec<u8>, NullBytes);

// Ergonomic extra: allow `&str` literals directly, without forcing
// `.to_string()` at every call site.
impl IntoExpr<Text> for &str {
    type Req = Nil;
    fn into_expr(self) -> Expr<Nil, Text> {
        Expr::from_kind(ExprKind::Value(Value::Text(self.to_string())))
    }
}

impl<S: SqlType> SqlType for crate::scope::Nullable<S> {
    type Native = Option<S::Native>;
}

impl<S: NullValue> IntoExpr<crate::scope::Nullable<S>> for Option<S::Native>
where
    S::Native: Into<Value>,
{
    type Req = Nil;
    fn into_expr(self) -> Expr<Nil, crate::scope::Nullable<S>> {
        Expr::from_kind(ExprKind::Value(match self {
            Some(v) => v.into(),
            None => S::NULL_VALUE,
        }))
    }
}

/// `count(*)`. A minimal, pragmatic start on aggregates — implemented via
/// the same `Raw` fragment machinery as `sql!{}` rather than a dedicated
/// `ExprKind::FunctionCall` variant, since a real function-call design
/// (covering `count(col)`, `sum(col)`, `avg(col)`, etc. generically) needs
/// the renderer to recursively render an inner `ExprKind` into a fragment,
/// which the current `Raw` text-with-`?`-placeholders shape doesn't support.
/// Deferred to whenever the broader aggregate-function API is designed
/// (tracked as Phase 3+ work) — this exists now only to make `GROUP BY` /
/// `HAVING` testable without blocking on that larger design.
pub fn count() -> Expr<Nil, BigInt> {
    Expr::from_kind(ExprKind::Raw {
        text: "count(*)".to_string(),
        params: Vec::new(),
    })
}

/// A named, typed placeholder: usable anywhere a value of type `S` is
/// expected (`.eq(placeholder::<Integer>("id"))`), rendering as a normal
/// bound parameter but resolved to a concrete value later, at
/// `Prepared::execute()` time, rather than when the query is built. Not
/// exported as part of the public "just write queries" surface — the
/// `prepare!{}` macro is the intended entry point, since it also generates
/// the typed `Params` struct that guarantees every placeholder actually
/// gets a value of the right type at execute time (closing the gap
/// Drizzle's own `sql.placeholder()` leaves: its `.execute()` takes an
/// untyped `Record<string, unknown>`, so a missing/misspelled key is only
/// caught at runtime).
#[doc(hidden)]
pub fn placeholder<S: SqlType>(name: &'static str) -> Expr<Nil, S> {
    Expr::from_kind(ExprKind::Value(Value::Placeholder(name)))
}
