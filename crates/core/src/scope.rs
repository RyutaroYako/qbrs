//! Type-level FROM/JOIN scope tracking.
//!
//! This is the load-bearing mechanism for the whole crate (see the design
//! plan): a flat cons-list of joined tables, with a frunk-style indexed
//! `Find` trait to look up whether/how a table appears in it. A naive
//! two-impl recursive `Contains<T>` does not compile (E0119, conflicting
//! impls) because the second impl is generic over `Head`, which overlaps
//! with the first impl once `Head` unifies with `TableSlot<T, N>`. The
//! `Here`/`There<I>` index parameter disambiguates the two impls
//! structurally instead of relying on shape-based negative reasoning.

use std::marker::PhantomData;

/// Marker trait for anything that represents a table in the schema.
/// `NAME` (and optionally `SCHEMA`) are what the SQL renderer uses to print
/// the actual identifier — kept as trait consts rather than derived from
/// the Rust type name so the derive macro can freely rename/alias tables.
pub trait Table: 'static {
    const NAME: &'static str;
    const SCHEMA: Option<&'static str> = None;
}

/// Marker trait for the two nullability states a table can have in a
/// query's scope, depending on how it was joined.
pub trait Nullability: 'static {}

/// The table is guaranteed present (INNER/CROSS join, or the FROM table).
pub struct NotNull;
impl Nullability for NotNull {}

/// The table may be absent (LEFT/RIGHT/FULL join introduced this
/// possibility for it).
pub struct MaybeNull;
impl Nullability for MaybeNull {}

/// Empty scope: no tables joined yet.
pub struct Nil;

/// A non-empty scope: `Head` joined with nullability tracked in
/// `TableSlot`, followed by the rest of the scope in `Tail`.
pub struct Cons<Head, Tail>(PhantomData<(Head, Tail)>);

/// One slot in the scope list: table `T`, with join-derived nullability `N`.
pub struct TableSlot<T: Table, N: Nullability>(PhantomData<(T, N)>);

/// Index marker: "found at the head of the list".
pub struct Here;

/// Index marker: "found further down the list, at index `I`".
pub struct There<I>(PhantomData<I>);

/// Proof that table `T` appears somewhere in a scope list, found at
/// compile-time-inferred position `Index`. `Index` is never spelled out
/// by callers — it's inferred, exactly like frunk's `Plucker`.
#[diagnostic::on_unimplemented(
    message = "`{T}` is not available in this query's scope",
    label = "add `.join(<table>, ..)` (or `.from(..)`) for `{T}` before referencing its columns here",
    note = "columns can only be referenced once their table has been joined into the current FROM/JOIN scope"
)]
pub trait Find<T: Table, Index> {
    /// The nullability `T` has in this scope (derived from how it was
    /// joined, not asserted manually).
    type Nullability: Nullability;
}

impl<T: Table, N: Nullability, Tail> Find<T, Here> for Cons<TableSlot<T, N>, Tail> {
    type Nullability = N;
}

impl<T: Table, Head, Tail, I> Find<T, There<I>> for Cons<Head, Tail>
where
    Tail: Find<T, I>,
{
    type Nullability = <Tail as Find<T, I>>::Nullability;
}

/// Concatenate two type-level lists. Used to union the `Req` (tables
/// referenced) of two expressions when they're combined via `.and()`/`.or()`
/// or a binary operator like `.eq()`.
pub trait Concat<Other> {
    type Output;
}

impl<Other> Concat<Other> for Nil {
    type Output = Other;
}

impl<Head, Tail: Concat<Other>, Other> Concat<Other> for Cons<Head, Tail> {
    type Output = Cons<Head, Tail::Output>;
}

/// Proof that scope `Self` contains every table named in `Req`, regardless
/// of each table's nullability. This is the bound used at the point an
/// already-built `Expr<Req, _>` is inserted into a query (`.filter()`,
/// `.join()`, `.select()`), and is what lets expressions be built as
/// portable, scope-agnostic values instead of via a scope-bound cursor
/// closure.
///
/// The second parameter `Idxs` mirrors `Find`'s own `Index` parameter: it
/// exists purely so the compiler has somewhere to put the per-element
/// lookup indices, and callers never name it explicitly (it's always
/// inferred, exactly like `Find`'s `Index`). A first attempt wrote this as
/// a single-parameter `Superset<Req>` with an existential `Idx` hidden in
/// the `where` clause (`S: Find<Head, Idx> + Superset<Tail>`) — that fails
/// to compile with E0207 ("type parameter `Idx` is not constrained")
/// because Rust has no existential quantification over impl generics; the
/// index has to be threaded through the trait's own parameter list instead,
/// the same way `Find`'s recursive case threads `I` through `There<I>`.
#[diagnostic::on_unimplemented(
    message = "this expression references a table that isn't in scope here",
    label = "requires {Req}, but the current query scope doesn't contain all of it"
)]
pub trait Superset<Req, Idxs> {}

impl<S> Superset<Nil, Nil> for S {}

impl<S, Head: Table, Tail, IdxHead, IdxsTail> Superset<Cons<Head, Tail>, Cons<IdxHead, IdxsTail>>
    for S
where
    S: Find<Head, IdxHead> + Superset<Tail, IdxsTail>,
{
}

/// Flip every table already in a scope to `MaybeNull`. Used by RIGHT/FULL
/// JOIN, which must retroactively make every previously-joined table
/// nullable (mirrors Drizzle's `AppendToNullabilityMap`), before adding the
/// newly-joined table's own slot.
pub trait MapNullable {
    type Output;
}

impl MapNullable for Nil {
    type Output = Nil;
}

impl<T: Table, N: Nullability, Tail: MapNullable> MapNullable for Cons<TableSlot<T, N>, Tail> {
    type Output = Cons<TableSlot<T, MaybeNull>, Tail::Output>;
}

/// Idempotently wrap a SQL type as nullable-or-not depending on a
/// `Nullability` marker, without double-wrapping an already-`Nullable<T>`
/// column. Column accessors compose this with `Find::Nullability` so that
/// NULL-ability is *derived* from join shape rather than requiring a manual
/// `.nullable()` assertion (diesel's documented wart).
pub trait WrapNullable<N: Nullability> {
    type Output;
}

/// Wraps a base SQL type as nullable. A distinct outer shape from any base
/// type, so it can have its own `WrapNullable<MaybeNull>` impl without
/// overlapping the per-base-type impls below.
pub struct Nullable<T>(PhantomData<T>);

// `NotNull` never changes the type, for *any* `T` (including `Nullable<T>`
// itself) — a single blanket impl is coherence-safe here because there is
// no second impl competing for the `NotNull` slot.
impl<T> WrapNullable<NotNull> for T {
    type Output = T;
}

// Idempotent: wrapping an already-nullable type as `MaybeNull` again is a
// no-op, not `Nullable<Nullable<T>>`.
impl<T> WrapNullable<MaybeNull> for Nullable<T> {
    type Output = Nullable<T>;
}

// IMPORTANT: there must be NO blanket `impl<T> WrapNullable<MaybeNull> for T`
// alongside the `Nullable<T>` impl above — the two would overlap (E0119) the
// same way a naive `Contains<T>` did, since a blanket impl over a fully
// generic `T` is considered by the coherence checker to potentially cover
// `Nullable<_>` too. Each concrete base SQL type (Integer, Text, Bool, ...)
// must instead get its own individual, non-generic `WrapNullable<MaybeNull>`
// impl — trivially generated by a macro over the closed base-type list, see
// `sql_leaf_type!` usage in Phase 1's `expr` module. This was caught during
// the Phase 0 spike: the design-review sketch in the plan document proposed
// a blanket `impl<T: SqlType> WrapNullable<MaybeNull> for T` which does NOT
// compile for exactly this reason.
