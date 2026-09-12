//! SQL types, columns, and the typed expression AST.
//!
//! `Expr<Req, S>` carries two purely phantom compile-time tags: `Req` (the
//! flat cons-list of tables this expression touches — see `scope::Superset`)
//! and `S` (its SQL type). The actual payload, `ExprKind`, is a plain closed
//! enum with no generics at all, so the renderer is never re-monomorphized
//! per query shape.

use std::marker::PhantomData;

use crate::scope::{Concat, Cons, MaybeNull, Nil, Table, WrapNullable};

mod sql_type {
    /// Sealed because the set really is closed: `Value` is a closed enum,
    /// so a type this crate can't render has nothing to be — and an open
    /// `SqlType` is what lets a schema crate pair a lying
    /// `WrapNullable<MaybeNull>` with a column type of its own.
    pub trait Sealed {}
}

/// A SQL scalar type. Implemented only by the closed set of leaf types
/// declared via `sql_leaf_type!` below, plus `Nullable<T>`.
pub trait SqlType: 'static + sql_type::Sealed {
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
    /// `excluded."col"` — the row an `INSERT` proposed, as `ON CONFLICT DO
    /// UPDATE` sees it.
    Excluded {
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
    /// `EXISTS (<subquery>)`. Held unrendered, because an `Expr` carries
    /// no dialect: rendering it here would let a subquery written for one
    /// dialect be filtered onto a statement of another.
    Exists {
        body: Box<crate::select::SelectBody>,
        selection: Vec<crate::render::SelectItem>,
        negated: bool,
    },
    /// `x IN (<subquery>)` / `x NOT IN (<subquery>)`. Held unrendered for
    /// the same reason `Exists` is: an `Expr` carries no dialect, and a
    /// subquery built for one dialect must not be filtered onto a statement
    /// of another.
    InSubquery {
        lhs: Box<ExprKind>,
        body: Box<crate::select::SelectBody>,
        selection: Vec<crate::render::SelectItem>,
        negated: bool,
    },
    /// `sql!{}`: authored text with a hole at each `?`, each hole holding an
    /// expression the renderer recurses into — so a column in a hole is
    /// quoted by the same code that quotes it anywhere else, and counts
    /// toward the fragment's `Req`.
    Template {
        head: String,
        rest: Vec<(ExprKind, String)>,
    },
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
    /// `string_agg(x, ', ')` and the two other spellings of it. Its own
    /// node rather than a `Func`, because the dialects disagree on the
    /// function's name and on where the separator goes, and an `Expr`
    /// carries no dialect to decide that when it is built. The separator is
    /// a `&'static str` written into the SQL rather than a bound value:
    /// MySQL's `SEPARATOR` takes a literal and rejects a parameter, so
    /// binding it would make the node unrenderable in one of the three.
    StringAgg {
        arg: Box<ExprKind>,
        separator: &'static str,
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
    /// Postgres array values: a `Vec<T>` binds as `T[]`, one variant per
    /// element type.
    TextArray(Vec<String>),
    NullTextArray,
    IntegerArray(Vec<i32>),
    NullIntegerArray,
    BigIntArray(Vec<i64>),
    NullBigIntArray,
    #[cfg(feature = "uuid")]
    UuidArray(Vec<uuid::Uuid>),
    #[cfg(feature = "uuid")]
    NullUuidArray,
    /// A JSON document, opaque to this crate: it binds and decodes, and the
    /// operators that look inside one go through `sql!{}`.
    #[cfg(feature = "json")]
    Json(serde_json::Value),
    #[cfg(feature = "json")]
    NullJson,
    /// A named placeholder in a `prepare!{}`-built query, not yet resolved
    /// to a concrete value. It rides the existing `Vec<Value>` parameter
    /// pipeline: rendering doesn't care what's *inside* a `Value`, only that
    /// there's one per placeholder position. `select::Prepared` substitutes
    /// real values before binding.
    Placeholder(&'static str),
}

/// What a `json` column stores of a document, which is the text as given:
/// `serde_json::Value`'s own `==` is structural and calls `0.0` and `-0.0`
/// one value, and key order is a build-wide choice of `serde_json`'s. Both
/// halves of the parameter index read a document through here, so what is
/// compared and what is hashed cannot drift apart.
#[cfg(feature = "json")]
fn json_as_written(value: &serde_json::Value) -> String {
    value.to_string()
}

impl Value {
    /// The SQL type this value came from, for the one error that has to name
    /// it: a column type enabled in this crate and not in the execution
    /// crate. Lives here because the feature-gated variants do.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::I32(_) | Value::NullI32 => "Integer",
            Value::I64(_) | Value::NullI64 => "BigInt",
            Value::F64(_) | Value::NullF64 => "Real",
            Value::Text(_) | Value::NullText => "Text",
            Value::Bool(_) | Value::NullBool => "Bool",
            Value::Bytes(_) | Value::NullBytes => "Bytes",
            Value::TextArray(_) | Value::NullTextArray => "TextArray",
            Value::IntegerArray(_) | Value::NullIntegerArray => "IntegerArray",
            Value::BigIntArray(_) | Value::NullBigIntArray => "BigIntArray",
            #[cfg(feature = "uuid")]
            Value::UuidArray(_) | Value::NullUuidArray => "UuidArray",
            #[cfg(feature = "json")]
            Value::Json(_) | Value::NullJson => "Json",
            Value::Placeholder(_) => "placeholder",
            #[cfg(feature = "chrono")]
            Value::Timestamptz(_) | Value::NullTimestamptz => "Timestamptz",
            #[cfg(feature = "chrono")]
            Value::Date(_) | Value::NullDate => "Date",
            #[cfg(feature = "uuid")]
            Value::Uuid(_) | Value::NullUuid => "Uuid",
            #[cfg(feature = "decimal")]
            Value::Numeric(_) | Value::NullNumeric => "Numeric",
        }
    }

    /// Whether these two values reach the database as the same parameter.
    /// Not `==`, which calls values equal that a column then stores apart:
    /// a `Decimal` compares by numeric value while Postgres's `numeric`
    /// keeps the scale it was handed, so `1.0` and `1.00` are equal and are
    /// stored as written; `0.0 == -0.0` while `double precision` keeps the
    /// sign. Sharing a parameter between two such values would bind the
    /// first one twice.
    pub(crate) fn binds_same_as(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::F64(a), Value::F64(b)) => a.to_bits() == b.to_bits(),
            #[cfg(feature = "decimal")]
            (Value::Numeric(a), Value::Numeric(b)) => a.serialize() == b.serialize(),
            #[cfg(feature = "json")]
            (Value::Json(a), Value::Json(b)) => json_as_written(a) == json_as_written(b),
            _ => self == other,
        }
    }

    /// Hashes what `binds_same_as` compares, so a statement can bucket the
    /// parameters it holds. Not a `Hash` impl: it answers `binds_same_as`,
    /// which is finer than `PartialEq`, and a `Hash` disagreeing with
    /// `PartialEq` breaks the contract one owes its callers. The caller
    /// supplies the hasher so the bucketing is keyed by the map's own
    /// `RandomState` — with a fixed seed, colliding text chosen by whoever
    /// supplies the values would walk the bucket the index exists to avoid.
    pub(crate) fn hash_into<H: std::hash::Hasher>(&self, hasher: &mut H) {
        use std::hash::Hash as _;
        std::mem::discriminant(self).hash(hasher);
        match self {
            Value::I32(v) => v.hash(hasher),
            Value::I64(v) => v.hash(hasher),
            Value::F64(v) => v.to_bits().hash(hasher),
            Value::Text(v) => v.hash(hasher),
            Value::Bool(v) => v.hash(hasher),
            Value::Bytes(v) => v.hash(hasher),
            Value::Placeholder(v) => v.hash(hasher),
            Value::TextArray(v) => v.hash(hasher),
            Value::IntegerArray(v) => v.hash(hasher),
            Value::BigIntArray(v) => v.hash(hasher),
            #[cfg(feature = "uuid")]
            Value::UuidArray(v) => v.hash(hasher),
            #[cfg(feature = "json")]
            Value::Json(v) => json_as_written(v).hash(hasher),
            #[cfg(feature = "chrono")]
            Value::Timestamptz(v) => v.hash(hasher),
            #[cfg(feature = "chrono")]
            Value::Date(v) => v.hash(hasher),
            #[cfg(feature = "uuid")]
            Value::Uuid(v) => v.hash(hasher),
            #[cfg(feature = "decimal")]
            Value::Numeric(v) => v.serialize().hash(hasher),
            Value::NullI32
            | Value::NullI64
            | Value::NullF64
            | Value::NullText
            | Value::NullBool
            | Value::NullBytes
            | Value::NullTextArray
            | Value::NullIntegerArray
            | Value::NullBigIntArray => {}
            #[cfg(feature = "uuid")]
            Value::NullUuidArray => {}
            #[cfg(feature = "json")]
            Value::NullJson => {}
            #[cfg(feature = "chrono")]
            Value::NullTimestamptz | Value::NullDate => {}
            #[cfg(feature = "uuid")]
            Value::NullUuid => {}
            #[cfg(feature = "decimal")]
            Value::NullNumeric => {}
        }
    }
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
value_from!(Vec<String>, TextArray);
value_from!(Vec<i32>, IntegerArray);
value_from!(Vec<i64>, BigIntArray);
#[cfg(feature = "uuid")]
value_from!(Vec<uuid::Uuid>, UuidArray);
#[cfg(feature = "json")]
value_from!(serde_json::Value, Json);

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
    message = "`{Self}` isn't a SQL expression",
    label = "a column, a literal, an aggregate, or a `sql!{{}}` fragment is; a `label!` name is not",
    note = "an `Option` isn't one either: asking about NULL is `.is_null()`, and assigning it is `null::<Text>()` — `= NULL` is never true in SQL"
)]
pub trait IntoExpr {
    /// The SQL type this expression has. An associated type rather than a
    /// parameter because every implementor has exactly one — a column its
    /// declared type, a literal its leaf type — which is what lets a
    /// mismatch report itself as `Comparable`/`AssignsTo` rather than as
    /// an inference failure on a type nobody wrote.
    type Sql: SqlType;
    type Req;
    fn into_expr(self) -> Expr<Self::Req, Self::Sql>;
}

impl<Req, S: SqlType> IntoExpr for Expr<Req, S> {
    type Sql = S;
    type Req = Req;
    fn into_expr(self) -> Expr<Req, S> {
        self
    }
}

/// What an expression can be assigned *to*. The value's type is `Self` and
/// the column's is the parameter, which is the direction assignment runs
/// in: a `Text` value goes into a `Nullable<Text>` column and a narrower
/// number into a wider one, never the reverse. (`Comparable` is the
/// symmetric, nullability-blind relation, and a comparison is symmetric.)
#[diagnostic::on_unimplemented(
    message = "a `{Self}` expression can't be assigned to a `{Column}` column",
    label = "the value has to fit the column: the same type, a narrower number, or a non-null value for a nullable column"
)]
pub trait AssignsTo<Column: SqlType>: SqlType {}

impl<T: SqlType> AssignsTo<T> for T {}
impl<T: SqlType> AssignsTo<crate::scope::Nullable<T>> for T {}

mod writable {
    /// Sealed like the other markers a derive emits: the door is
    /// `#[doc(hidden)]`, so writing to a generated column is something a
    /// caller can only do on purpose, never by forgetting an attribute.
    pub trait Sealed {}
}

#[doc(hidden)]
pub use writable::Sealed as WritableSealed;

/// A column a statement may write to: `#[derive(Table)]` emits this for
/// every column except the generated and primary-key ones, which are the
/// same set `*Update` leaves out.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a column a statement can assign to",
    label = "a primary-key or generated column is the database's to write, which is why `*Update` leaves it out too"
)]
pub trait Writable: WritableSealed {}

/// A column's compile-time identity. One implementor per column in the
/// schema, generated by `#[derive(Table)]` (and by `with!{}` for a CTE's
/// pseudo-columns), which is what lets a column be a *key* — two columns of
/// the same table and SQL type are still distinct types here, so a row can
/// be indexed by column without ambiguity.
pub trait ColumnKey: crate::row::Spelled + Copy + 'static {
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

impl<C: ColumnKey> IntoExpr for Column<C> {
    type Sql = C::Sql;
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
/// `LabelExt::label` exists to resolve.
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

impl<K, Req, S: SqlType> IntoExpr for Keyed<K, Req, S> {
    type Sql = S;
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
    note = "nullability doesn't matter here: a `Nullable<T>` compares with a `T`",
    note = "an unannotated `vec![1, 2]` is an `integer[]`, since that is what an integer literal defaults to — a `bytea` takes `vec![1u8, 2]`"
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

macro_rules! assigns_across {
    ($($from:ty => $to:ty),+ $(,)?) => {
        $(
            impl AssignsTo<$to> for $from {}
            impl AssignsTo<crate::scope::Nullable<$to>> for $from {}
            impl AssignsTo<crate::scope::Nullable<$to>> for crate::scope::Nullable<$from> {}
        )+
    };
}
// Widening only: an `Integer` expression fits a `BigInt` column, not the
// other way round.
assigns_across!(
    Integer => BigInt,
    Integer => Real,
    BigInt => Real,
);

#[cfg(feature = "decimal")]
assigns_across!(
    Integer => Numeric,
    BigInt => Numeric,
    Real => Numeric,
);

// A money column is compared against a literal more than it is compared
// against another money column, and every database this crate speaks
// compares numeric with the integers and floats.
#[cfg(feature = "decimal")]
comparable_across!(
    Numeric => Integer,
    Integer => Numeric,
    Numeric => BigInt,
    BigInt => Numeric,
    Numeric => Real,
    Real => Numeric,
);

/// A caller-declared output-column name, generated by `label!{}`. Being a
/// marker over `Named` is what keeps `label!{}` the only way to make one,
/// and `Named::NAME` the only place the name is written.
pub trait LabelKey: crate::row::Spelled + Copy + 'static {}

/// An expression that has stated what it decodes to but has no name: the
/// anonymous `Keyed`, and so selectable, labellable and usable in a slot on
/// exactly the same terms as any other keyed expression — except that
/// `Anon` is not `Spelled`, so it can't be looked up or matched by name.
pub type Declared<Req, S> = Keyed<crate::row::Anon, Req, S>;

impl<Req, S: SqlType> Expr<Req, S> {
    /// States what this expression decodes to, which is what makes it
    /// selectable: `S` was inferred from whatever built the expression, and
    /// an inference can contradict the join the query actually has.
    pub fn decodes_as<S2: SqlType>(self) -> Declared<Req, S2> {
        Keyed {
            kind: self.kind,
            _marker: PhantomData,
        }
    }
}

/// A selected item filed under a `LabelKey` instead of under its own
/// identity, and rendered with that name as its `AS`.
pub struct Labeled<K, Inner> {
    pub(crate) inner: Inner,
    _key: PhantomData<fn() -> K>,
}

impl<K, Inner: Clone> Clone for Labeled<K, Inner> {
    fn clone(&self) -> Self {
        Labeled::new(self.inner.clone())
    }
}

impl<K, Inner> Labeled<K, Inner> {
    pub(crate) fn new(inner: Inner) -> Self {
        Labeled {
            inner,
            _key: PhantomData,
        }
    }
}

/// Files a selected item under a declared name: the way to select the same
/// expression twice, and the way out of two tables' same-named columns
/// colliding.
pub trait LabelExt: Sized {
    fn label<K: LabelKey>(self, _key: K) -> Labeled<K, Self> {
        Labeled::new(self)
    }
}
impl<C: ColumnKey> LabelExt for Column<C> {}
impl<K, Req, S: SqlType> LabelExt for Keyed<K, Req, S> {}
// A bare `Expr` isn't selectable — it has to state its decoded type first —
// but labelling one has to *reach* that rule to report it. Without this
// impl, `.label(..)` on an inferred expression is a missing method and the
// sentence about `.decodes_as::<..>()` is never printed.
impl<Req, S: SqlType> LabelExt for Expr<Req, S> {}

/// Comparison/boolean-combinator methods, blanket-implemented for anything
/// convertible to a typed expression (columns, literals, and `Expr` itself).
/// Kept separate from `IntoExpr` so one blanket impl can serve all three.
// Every combinator here consumes `self`, this crate's builders being
// by-value throughout; `is_null`/`is_in` are combinators, not predicates on
// an existing value.
#[allow(clippy::wrong_self_convention)]
pub trait ExprMethods: IntoExpr + Sized {
    fn eq<Rhs: IntoExpr>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Self::Sql: Comparable<Rhs::Sql>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Eq, self, rhs)
    }

    fn ne<Rhs: IntoExpr>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Self::Sql: Comparable<Rhs::Sql>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Ne, self, rhs)
    }

    fn lt<Rhs: IntoExpr>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Self::Sql: Comparable<Rhs::Sql>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Lt, self, rhs)
    }

    fn lte<Rhs: IntoExpr>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Self::Sql: Comparable<Rhs::Sql>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Lte, self, rhs)
    }

    fn gt<Rhs: IntoExpr>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Self::Sql: Comparable<Rhs::Sql>,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Gt, self, rhs)
    }

    fn gte<Rhs: IntoExpr>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Self::Sql: Comparable<Rhs::Sql>,
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

    /// `a AND b`. The boolean requirement is on the method rather than on
    /// the receiver's type, so every spelling a condition has — a
    /// comparison, a `sql!` fragment, a `Nullable<Bool>` column — combines
    /// with every other, the way `.filter` accepts them all.
    fn and<Rhs: IntoExpr>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Self::Sql: BoolLike,
        Rhs::Sql: BoolLike,
        Self::Req: Concat<Rhs::Req>,
    {
        Expr::from_kind(ExprKind::And(
            Box::new(self.into_expr().kind),
            Box::new(rhs.into_expr().kind),
        ))
    }

    /// `a OR b` — see `and`.
    fn or<Rhs: IntoExpr>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Self::Sql: BoolLike,
        Rhs::Sql: BoolLike,
        Self::Req: Concat<Rhs::Req>,
    {
        Expr::from_kind(ExprKind::Or(
            Box::new(self.into_expr().kind),
            Box::new(rhs.into_expr().kind),
        ))
    }

    /// `x LIKE 'pattern'`. The text requirement is on the method rather
    /// than on a trait of its own, so a non-text operand reports `TextLike`
    /// instead of a missing method.
    fn like<Rhs: IntoExpr>(self, rhs: Rhs) -> Expr<<Self::Req as Concat<Rhs::Req>>::Output, Bool>
    where
        Self::Sql: TextLike,
        Rhs::Sql: TextLike,
        Self::Req: Concat<Rhs::Req>,
    {
        bin_op(BinOp::Like, self, rhs)
    }

    /// `x IN (a, b, ..)` over a runtime-length list of literals, each bound
    /// as its own parameter. An empty list renders `FALSE`.
    ///
    /// **Known limitation**: the list holds values, not expressions — a
    /// column reference on the right needs the table it belongs to folded
    /// into `Req`, which is the same design `sql!{}` covers today.
    fn is_in<I>(self, values: I) -> Expr<Self::Req, Bool>
    where
        I: IntoIterator,
        I::Item: IntoExpr<Req = Nil>,
        Self::Sql: Comparable<<I::Item as IntoExpr>::Sql>,
    {
        let values: Vec<ExprKind> = values.into_iter().map(|v| v.into_expr().kind).collect();
        // `IN ()` is not SQL, and matching nothing is what it would mean.
        if values.is_empty() {
            return Expr::from_kind(ExprKind::Always(false));
        }
        Expr::from_kind(ExprKind::InList {
            expr: Box::new(self.into_expr().kind),
            values,
        })
    }
}

/// True when any of the conditions is. Takes a runtime-length collection,
/// the way `is_in` takes a runtime-length list of values, so the `OR` a
/// search box needs doesn't have to be folded by hand — folding one by one
/// grows `Req` and stops type-checking after the first pair. An empty
/// collection matches nothing, which is what `is_in([])` says too.
pub fn any_of<Req, C: IntoExpr<Req = Req>>(conds: impl IntoIterator<Item = C>) -> Expr<Req, Bool>
where
    C::Sql: BoolLike,
{
    combine(conds, false)
}

/// True when all of them are. An empty collection matches everything, which
/// is what a `WHERE` with no conditions does.
pub fn all_of<Req, C: IntoExpr<Req = Req>>(conds: impl IntoIterator<Item = C>) -> Expr<Req, Bool>
where
    C::Sql: BoolLike,
{
    combine(conds, true)
}

fn combine<Req, C: IntoExpr<Req = Req>>(
    conds: impl IntoIterator<Item = C>,
    all: bool,
) -> Expr<Req, Bool>
where
    C::Sql: BoolLike,
{
    Expr::from_kind(fold_conditions(
        conds.into_iter().map(|c| c.into_expr().kind),
        all,
    ))
}

/// AND- or OR-folds conditions, answering `TRUE`/`FALSE` for an empty
/// collection — "all of nothing" matches everything, "any of nothing"
/// matches nothing. Shared with `select::Predicate`, which folds the same
/// way once the scope requirement is discharged.
pub(crate) fn fold_conditions(kinds: impl IntoIterator<Item = ExprKind>, all: bool) -> ExprKind {
    let mut folded: Option<ExprKind> = None;
    for kind in kinds {
        folded = Some(match folded {
            None => kind,
            Some(acc) if all => ExprKind::And(Box::new(acc), Box::new(kind)),
            Some(acc) => ExprKind::Or(Box::new(acc), Box::new(kind)),
        });
    }
    folded.unwrap_or(ExprKind::Always(all))
}

impl<T: IntoExpr> ExprMethods for T {}

fn bin_op<Lhs, Rhs>(
    op: BinOp,
    lhs: Lhs,
    rhs: Rhs,
) -> Expr<<Lhs::Req as Concat<Rhs::Req>>::Output, Bool>
where
    Lhs: IntoExpr,
    Rhs: IntoExpr,
    Lhs::Req: Concat<Rhs::Req>,
{
    Expr::from_kind(ExprKind::BinOp {
        op,
        lhs: Box::new(lhs.into_expr().kind),
        rhs: Box::new(rhs.into_expr().kind),
    })
}

/// What `LIKE` accepts: a text expression, nullable or not. Its own marker
/// rather than `Comparable<Text>` so the failure says what the operator
/// needs instead of talking about comparison.
#[diagnostic::on_unimplemented(
    message = "`LIKE` needs a text expression, and `{Self}` isn't one",
    label = "only `Text` and `Nullable<Text>` columns and expressions accept `.like(..)`"
)]
pub trait TextLike: SqlType {}

impl TextLike for Text {}
impl TextLike for crate::scope::Nullable<Text> {}

/// What a `WHERE`/`HAVING`/`ON` clause accepts. `Nullable<Bool>` belongs
/// here because SQL takes it: a NULL condition selects no row, which is
/// the same answer `IS NOT TRUE` would give.
#[diagnostic::on_unimplemented(
    message = "a condition has to be a boolean expression, and `{Self}` isn't one",
    label = "expected `Bool` or `Nullable<Bool>`"
)]
pub trait BoolLike: SqlType {}

impl BoolLike for Bool {}
impl BoolLike for crate::scope::Nullable<Bool> {}

/// `!condition`, not `condition.not()`: the standard `Not` trait reads more
/// naturally at call sites than a same-named inherent method. Implemented
/// for all three spellings `.filter` takes, each keeping its own type —
/// `NOT` of a `Nullable<Bool>` is still nullable, and a `sql!` fragment
/// stays the same fragment.
impl<Req, S: BoolLike> std::ops::Not for Expr<Req, S> {
    type Output = Expr<Req, S>;
    fn not(self) -> Self::Output {
        Expr::from_kind(ExprKind::Not(Box::new(self.kind)))
    }
}

impl<K, Req, S: BoolLike> std::ops::Not for Keyed<K, Req, S> {
    type Output = Keyed<K, Req, S>;
    fn not(self) -> Self::Output {
        Keyed::from_kind(ExprKind::Not(Box::new(self.kind)))
    }
}

impl<C: ColumnKey> std::ops::Not for Column<C>
where
    C::Sql: BoolLike,
{
    type Output = Expr<Cons<C::Table, Nil>, C::Sql>;
    fn not(self) -> Self::Output {
        Expr::from_kind(ExprKind::Not(Box::new(self.into_expr().kind)))
    }
}

/// A base SQL type's typed NULL — see `Value::NullI32` etc. for why this
/// can't just be a single untyped `Value::Null`.
pub trait NullValue: SqlType {
    const NULL_VALUE: Value;
}

/// A typed SQL `NULL`, for the one position an `Option` can't say it:
/// `SET column = NULL` assigns, and an assignment has no `Option` to be
/// `None`. `null::<Text>()` is `Nullable<Text>`, so only a nullable column
/// accepts it.
pub fn null<S: NullValue>() -> Expr<Nil, crate::scope::Nullable<S>> {
    Expr::from_kind(ExprKind::Value(S::NULL_VALUE))
}

/// Declares a leaf (base) SQL type: the marker struct, its `SqlType` impl,
/// its `WrapNullable<MaybeNull>` impl, and `IntoExpr` from its native Rust
/// type. One concrete, non-generic impl per type — a blanket
/// `impl<T: SqlType> WrapNullable<MaybeNull> for T` would conflict with
/// `Nullable<T>`'s own impl (see `scope::WrapNullable`).
mod raw_arg {
    /// Sealed for the reason `select::ColumnList` is: `Req` is a free
    /// parameter, and a slot's value can be delegated to a real column — so
    /// a hand-written impl could claim `Nil` while naming a table, which is
    /// exactly the scope check a `sql!` slot exists to keep.
    pub trait Sealed {}
    impl<T: super::IntoExpr> Sealed for T {}
}

macro_rules! sql_leaf_type {
    ($name:ident, $native:ty, $null_variant:ident) => {
        pub struct $name;

        impl sql_type::Sealed for $name {}

        impl SqlType for $name {
            type Native = $native;
        }

        impl crate::scope::wrap::Sealed<MaybeNull> for $name {}

        impl crate::scope::WrapNullable<MaybeNull> for $name {
            type Output = crate::scope::Nullable<$name>;
        }

        impl IntoExpr for $native {
            type Sql = $name;
            type Req = Nil;
            fn into_expr(self) -> Expr<Nil, $name> {
                Expr::from_kind(ExprKind::Value(Value::from(self)))
            }
        }

        impl crate::select::SingleColumn for $native {}
        impl crate::select::SingleColumn for ::std::option::Option<$native> {}

        impl NullValue for $name {
            const NULL_VALUE: Value = Value::$null_variant;
        }

        impl crate::insert::IntoColumnValue<$native> for $native {
            fn into_column_value(self) -> $native {
                self
            }
        }

        impl crate::insert::IntoColumnValue<::std::option::Option<$native>> for $native {
            fn into_column_value(self) -> ::std::option::Option<$native> {
                ::std::option::Option::Some(self)
            }
        }

        impl crate::insert::IntoColumnValue<::std::option::Option<$native>>
            for ::std::option::Option<$native>
        {
            fn into_column_value(self) -> ::std::option::Option<$native> {
                self
            }
        }

        impl crate::insert::IntoColumnValue<crate::insert::Defaultable<$native>> for $native {
            fn into_column_value(self) -> crate::insert::Defaultable<$native> {
                crate::insert::Defaultable::Value(self)
            }
        }

        impl crate::insert::IntoColumnValue<crate::insert::Defaultable<$native>>
            for ::std::option::Option<$native>
        {
            fn into_column_value(self) -> crate::insert::Defaultable<$native> {
                match self {
                    ::std::option::Option::Some(v) => crate::insert::Defaultable::Value(v),
                    ::std::option::Option::None => crate::insert::Defaultable::Default,
                }
            }
        }

        impl
            crate::insert::IntoColumnValue<
                crate::insert::Defaultable<::std::option::Option<$native>>,
            > for $native
        {
            fn into_column_value(
                self,
            ) -> crate::insert::Defaultable<::std::option::Option<$native>> {
                crate::insert::Defaultable::Value(::std::option::Option::Some(self))
            }
        }

        impl
            crate::insert::IntoColumnValue<
                crate::insert::Defaultable<::std::option::Option<$native>>,
            > for ::std::option::Option<$native>
        {
            fn into_column_value(
                self,
            ) -> crate::insert::Defaultable<::std::option::Option<$native>> {
                match self {
                    ::std::option::Option::Some(v) => {
                        crate::insert::Defaultable::Value(::std::option::Option::Some(v))
                    }
                    ::std::option::Option::None => crate::insert::Defaultable::Default,
                }
            }
        }

        impl raw_arg::Sealed for ::std::option::Option<$native> {}

        impl RawArg for ::std::option::Option<$native> {
            type Req = Nil;
            fn into_raw_arg(self) -> RawSlot {
                RawSlot(ExprKind::Value(Value::from(self)))
            }
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

// Postgres arrays, over the four element types a schema reaches for.
//
// **Known limitations**: `BOOLEAN[]`, `DOUBLE PRECISION[]`, `TIMESTAMPTZ[]`
// and `NUMERIC[]` have no marker, and neither does an array whose elements
// can be NULL — a `Vec<T>` column decodes every element, so a row holding
// one fails to decode rather than arriving as `None`. Both wait for a
// schema that needs them. Arrays are Postgres's alone; an `Expr` carries no
// dialect, so rendering one for MySQL or SQLite is a bind their driver
// refuses rather than a compile error.
sql_leaf_type!(TextArray, Vec<String>, NullTextArray);
sql_leaf_type!(IntegerArray, Vec<i32>, NullIntegerArray);
sql_leaf_type!(BigIntArray, Vec<i64>, NullBigIntArray);
#[cfg(feature = "uuid")]
sql_leaf_type!(UuidArray, Vec<uuid::Uuid>, NullUuidArray);

// A JSON document, Postgres's `jsonb`. Opaque: it goes in and comes back,
// and `->`, `->>`, `@>` and the rest of the operators that look inside one
// are deferred to `sql!{}` rather than half-built.
//
// **Known limitations**: a `json` column binds and decodes through this
// marker too, since the two are one wire format and one Rust type, but
// `json` has neither an equality nor an ordering operator — `.eq(..)`,
// `.asc()` and `GROUP BY` on one compile here and are rejected by the
// server. Which of the two a column is is a fact about the schema that
// nothing in a query can read, so it is `jsonb` that this marker claims.
#[cfg(feature = "json")]
sql_leaf_type!(Json, serde_json::Value, NullJson);

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
impl IntoExpr for &String {
    type Sql = Text;
    type Req = Nil;
    fn into_expr(self) -> Expr<Nil, Text> {
        Expr::from_kind(ExprKind::Value(Value::Text(self.clone())))
    }
}

impl IntoExpr for &str {
    type Sql = Text;
    type Req = Nil;
    fn into_expr(self) -> Expr<Nil, Text> {
        Expr::from_kind(ExprKind::Value(Value::Text(self.to_string())))
    }
}

/// Text's borrowed forms, at every slot a `String` column has. The leaf
/// macro can't generate these: only `Text` has a borrowed spelling.
macro_rules! text_column_value {
    ($borrowed:ty) => {
        impl crate::insert::IntoColumnValue<String> for $borrowed {
            fn into_column_value(self) -> String {
                self.to_string()
            }
        }

        impl crate::insert::IntoColumnValue<Option<String>> for $borrowed {
            fn into_column_value(self) -> Option<String> {
                Some(self.to_string())
            }
        }

        impl crate::insert::IntoColumnValue<crate::insert::Defaultable<String>> for $borrowed {
            fn into_column_value(self) -> crate::insert::Defaultable<String> {
                crate::insert::Defaultable::Value(self.to_string())
            }
        }

        impl crate::insert::IntoColumnValue<crate::insert::Defaultable<Option<String>>>
            for $borrowed
        {
            fn into_column_value(self) -> crate::insert::Defaultable<Option<String>> {
                crate::insert::Defaultable::Value(Some(self.to_string()))
            }
        }
    };
}

text_column_value!(&str);
text_column_value!(&String);

impl<S: SqlType> sql_type::Sealed for crate::scope::Nullable<S> {}

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

/// What `min`/`max` accept: a type the databases order. Its own marker for
/// the reason `Summable` is one — `WrapNullable<MaybeNull>`, which stood
/// here before, is implemented for every leaf type, so it gated nothing and
/// `max(bool_column)` rendered SQL Postgres has no aggregate for.
///
/// The set is Postgres's, the narrowest of the three: no `boolean`, no
/// `bytea`, and no `uuid` before PG 18.
#[diagnostic::on_unimplemented(
    message = "`min`/`max` need an ordered expression, and `{Self}` isn't one",
    label = "numbers, text, and dates/timestamps are ordered; booleans, bytes and UUIDs are not — `bool_or`/`bool_and` are the aggregate a flag wants, and aren't built yet"
)]
pub trait Ordered: SqlType + WrapNullable<MaybeNull> {}

impl Ordered for Integer {}
impl Ordered for BigInt {}
impl Ordered for Real {}
impl Ordered for Text {}
#[cfg(feature = "decimal")]
impl Ordered for Numeric {}
#[cfg(feature = "chrono")]
impl Ordered for Timestamptz {}
#[cfg(feature = "chrono")]
impl Ordered for Date {}

/// A nullable column orders like its base type — the NULLs sort, they don't
/// stop the aggregate from existing.
impl<S: Ordered> Ordered for crate::scope::Nullable<S> {}

/// What `sum(..)` of a column decodes to. `sum` is NULL over zero rows, so
/// every result is nullable however the column was declared.
/// `CAST` keeps the widened type a database picks for a sum inside the
/// closed set of types this crate has: Postgres returns `numeric` for
/// `sum(bigint)` and `avg(int)`, neither of which has a native here.
#[diagnostic::on_unimplemented(
    message = "`sum`/`avg` need a numeric expression, and `{Self}` isn't one",
    label = "only `Integer`, `BigInt` and `Real` columns and expressions (and `Numeric`, with the `decimal` feature) can be summed or averaged"
)]
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
// `sum(numeric)` stays numeric, so it needs no cast; `avg` decodes as `f64`
// for every column type, so a numeric one is cast like the rest.
#[cfg(feature = "decimal")]
impl Summable for Numeric {
    type Sum = crate::scope::Nullable<Numeric>;
    const SUM_CAST: Option<CastTarget> = None;
    const AVG_CAST: Option<CastTarget> = Some(CastTarget::Double);
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

// An aggregate is filed under the column it aggregates: `sum(orders::total)`
// reads back as `total`, and matches a CTE or DTO field of that name.
//
// **Known limitation**: by *name*, not by key — `Agg<Sum, total>` is its own
// key type, so `row.get(sum(orders::total))` and `#[derive(FromRow)]` find
// it and the generated `row.total()` accessor does not.
#[doc(hidden)]
impl<Op, C: crate::row::Named> crate::row::NamedSealed for Agg<Op, C> {}

#[doc(hidden)]
impl<Op: 'static, C: crate::row::Named + 'static> crate::row::Named for Agg<Op, C> {
    type Name = <C as crate::row::Named>::Name;
    const NAME: &'static str = <C as crate::row::Named>::NAME;
}

#[doc(hidden)]
impl<Op: 'static, C: crate::row::Spelled + 'static> crate::row::Spelled for Agg<Op, C> {}

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
    Ordered,
    None,
    "`min(column)`. NULL over zero rows."
);
aggregate!(
    Max,
    max,
    "max",
    <C::Sql as WrapNullable<MaybeNull>>::Output,
    Ordered,
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

/// What `string_agg` accepts. Postgres defines it for `text` and for
/// `bytea` — and the `bytea` one concatenates bytes and returns `bytea`,
/// which is a different question than the one this asks — so text is the
/// set, as it is the set the other two dialects coerce their arguments
/// into anyway.
#[diagnostic::on_unimplemented(
    message = "`string_agg` concatenates text, and `{Self}` isn't text",
    label = "reach for a cast, or a raw fragment, in front of a column that isn't"
)]
pub trait Concatenable: SqlType {}

impl Concatenable for Text {}

/// A nullable column concatenates like its base type: `string_agg` skips
/// the NULLs rather than being undefined over them.
impl<S: Concatenable> Concatenable for crate::scope::Nullable<S> {}

/// `string_agg(column, ", ")` — a group's values run together, separated.
/// NULL over zero rows, and over a group whose every value is NULL.
///
/// The separator binds like any other value under Postgres and SQLite,
/// which take it as an ordinary argument. MySQL's grammar takes a literal
/// after `SEPARATOR` and rejects a parameter, so there alone it is written
/// into the SQL and escaped — which is why it is a `&'static str` at all,
/// and why a MySQL session running `NO_BACKSLASH_ESCAPES` renders a
/// separator containing a backslash as more backslashes than were asked
/// for. `&'static str` is a nudge and not a guarantee, since `Box::leak`
/// reaches it; the guarantee is that the two dialects this crate executes
/// never write it out at all.
///
/// Two `string_agg`s over one column key alike, since the separator is not
/// part of the key — give one a `label!{}` name to read both back.
///
/// **Known limitation**: no `ORDER BY` inside the call
/// (`string_agg(x, ',' ORDER BY x)`) and no `DISTINCT`. Ordering inside an
/// aggregate reached SQLite only in 3.44, past the 3.39 this crate targets,
/// and MySQL spells it before the separator rather than after the
/// argument — three spellings of a clause two of the dialects would have
/// to be told to skip. Reach for `sql!{}` where the order matters.
pub struct StringAgg;

/// `string_agg(column, ", ")`. See [`StringAgg`].
pub fn string_agg<C: ColumnKey>(
    _column: Column<C>,
    separator: &'static str,
) -> Keyed<Agg<StringAgg, C>, Cons<C::Table, Nil>, crate::scope::Nullable<Text>>
where
    C::Sql: Concatenable,
{
    Keyed::from_kind(ExprKind::StringAgg {
        arg: Box::new(ExprKind::Column {
            table: <C::Table as Table>::NAME,
            name: <C as crate::row::Named>::NAME,
        }),
        separator,
    })
}

/// One `?` slot of a `sql!{}` fragment: every expression, plus the `Option`
/// a request field already holds — a slot is the one place a NULL arrives
/// as data rather than as a written `null::<..>()`. A slot that isn't one
/// reports `IntoExpr`, since that is the bound this one is built on.
pub trait RawArg: raw_arg::Sealed {
    type Req;
    #[doc(hidden)]
    fn into_raw_arg(self) -> RawSlot;
}

impl<T: IntoExpr> RawArg for T {
    type Req = T::Req;
    fn into_raw_arg(self) -> RawSlot {
        RawSlot(self.into_expr().kind)
    }
}

/// What a slot holds, opaque outside this crate: the `RawArg` impls are the
/// only way to make one, so a slot always holds something the renderer can
/// write.
pub struct RawSlot(ExprKind);

impl RawSlot {
    fn into_kind(self) -> ExprKind {
        self.0
    }
}

/// The whole slot list of one `sql!{}`, whose `Req` is every table its
/// slots name.
#[diagnostic::on_unimplemented(
    message = "a `sql!` fragment takes at most 8 `?` slots",
    label = "split the fragment, or fold part of it into the builder"
)]
pub trait RawArgs {
    type Req;
    #[doc(hidden)]
    fn into_raw_args(self) -> Vec<RawSlot>;
}

impl RawArgs for () {
    type Req = Nil;
    fn into_raw_args(self) -> Vec<RawSlot> {
        Vec::new()
    }
}

macro_rules! raw_args_tuple {
    ($head:ident $(, $rest:ident)*) => {
        #[allow(non_snake_case)]
        impl<$head: RawArg $(, $rest: RawArg)*> RawArgs for ($head, $($rest,)*)
        where
            ($($rest,)*): RawArgs,
            $head::Req: Concat<<($($rest,)*) as RawArgs>::Req>,
        {
            type Req = <$head::Req as Concat<<($($rest,)*) as RawArgs>::Req>>::Output;
            fn into_raw_args(self) -> Vec<RawSlot> {
                let ($head, $($rest,)*) = self;
                let mut kinds = ::std::vec![RawArg::into_raw_arg($head)];
                kinds.extend(RawArgs::into_raw_args(($($rest,)*)));
                kinds
            }
        }
    };
}
raw_args_tuple!(A);
raw_args_tuple!(A, B);
raw_args_tuple!(A, B, C);
raw_args_tuple!(A, B, C, D);
raw_args_tuple!(A, B, C, D, E);
raw_args_tuple!(A, B, C, D, E, F);
raw_args_tuple!(A, B, C, D, E, F, G);
raw_args_tuple!(A, B, C, D, E, F, G, H);

/// Counts the `?` slots in a `sql!` text, so the macro can compare that
/// count with the number of arguments it was handed while both are still
/// constants.
#[doc(hidden)]
pub const fn placeholder_count(sql: &str) -> usize {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut count = 0;
    while i < bytes.len() {
        if bytes[i] == b'?' {
            count += 1;
        }
        i += 1;
    }
    count
}

/// The one door into `ExprKind` from outside the crate, and the only shape
/// that needs one: `sql!{}` expands in the caller's. `Req` is the union of
/// the slots' own, so a fragment carries exactly the tables its `?`s name.
/// Reached through `sql!`, which is what checks that every `?` has a value.
#[doc(hidden)]
pub fn raw_expr<S: SqlType, Args: RawArgs>(
    sql: &'static str,
    args: Args,
) -> Declared<Args::Req, S> {
    Keyed::from_kind(template(sql, args.into_raw_args()))
}

/// Splits authored text on its `?` slots and pairs each with its argument.
/// `sql!` checks the two counts against each other while both are still
/// constants, which is why leftovers here can't happen through it.
fn template(sql: &'static str, args: Vec<RawSlot>) -> ExprKind {
    let mut pieces: Vec<String> = vec![String::new()];
    for c in sql.chars() {
        match c {
            '?' => pieces.push(String::new()),
            c => pieces.last_mut().expect("one piece to start").push(c),
        }
    }

    let mut pieces = pieces.into_iter();
    let head = pieces.next().unwrap_or_default();
    let rest = args
        .into_iter()
        .map(RawSlot::into_kind)
        .zip(pieces)
        .collect();
    ExprKind::Template { head, rest }
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
