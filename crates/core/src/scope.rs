//! Type-level FROM/JOIN scope tracking.
//!
//! This is the load-bearing mechanism for the whole crate: a flat cons-list
//! of joined tables, with a frunk-style indexed `Find` trait to look up
//! whether/how a table appears in it.

use std::marker::PhantomData;

/// Marker trait for anything that represents a table in a query's scope.
/// `NAME` is what the SQL renderer prints, kept as a trait const rather than
/// derived from the Rust type name so the derive macro can freely rename
/// tables.
///
/// **Known limitation**: no schema qualification. A table is rendered
/// bare, so `search_path` decides which one it is.
pub trait Table: 'static {
    const NAME: &'static str;
}

/// A table that exists in the schema, as opposed to one a query brings into
/// being. `.from()` and the joins take these; a CTE's pseudo-table is
/// reached through its `Cte` binding instead, so selecting from a CTE that
/// was never bound cannot be written.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a schema table",
    label = "a `with!{{}}` pseudo-table is entered through its `cte::with(..)` binding",
    note = "pass the `cte::with(..)` binding itself to `.from(..)`/`.inner_join(..)`: binding a CTE is what puts it in scope"
)]
pub trait BaseTable: Table + private::Sealed {}

mod private {
    /// Re-exported `#[doc(hidden)]` rather than kept private, because
    /// `#[derive(Table)]` emits the impl in the caller's crate. `with!` emits
    /// none, which keeps a CTE's pseudo-table out of `.from(..)` except
    /// through its binding.
    pub trait Sealed {}
}

#[doc(hidden)]
pub use private::Sealed as BaseTableSealed;

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
/// `TableSlot`, followed by the rest of the scope in `Tail`. Most recently
/// joined table first, which is the order a hand-written `Scope` alias has
/// to be spelled in.
///
/// **Known limitation**: a table appears at most once, and uniqueness is by
/// marker type, not by SQL name. Joining one marker twice is reported later,
/// as an inference ambiguity the first time a column of it is referenced;
/// two *different* markers that share a `Table::NAME` are not caught at all
/// and render `FROM "t" JOIN "t"`. Two schemas must not name the same table.
pub struct Cons<Head, Tail>(PhantomData<(Head, Tail)>);

/// One occurrence of a table in the scope list: table `T`, with the
/// nullability `N` its join gave it.
pub struct TableSlot<T: Table, N: Nullability>(PhantomData<(T, N)>);

/// Index marker: "found at the head of the list".
pub struct Here;

/// Index marker: "found further down the list, at index `I`".
pub struct There<I>(PhantomData<I>);

/// The 1-based position an index names, which is what SQL's `ORDER BY <n>`
/// on a set operation takes. The index is already computed by `row::Field`;
/// this reads it, so a set operation can be ordered by a column rather than
/// by a number nothing checks.
pub trait Position {
    const POSITION: u32;
}

impl Position for Here {
    const POSITION: u32 = 1;
}

impl<I: Position> Position for There<I> {
    const POSITION: u32 = I::POSITION + 1;
}

/// Each seal carries the parameters of the trait it seals, rather than
/// sealing `Self` alone: `Find<T, _>` and `row::Field<K, _>` take the
/// caller's own marker as a bare parameter, so the orphan rule licenses
/// `impl Find<orders::Table, Here> for <a scope without orders>`, which
/// silences the error this crate exists to produce.
///
/// A private associated *type* (`type Proof: Sealed`) does not seal either:
/// path privacy does not reach through projection, so
/// `<Nil as Superset<Nil, Nil>>::Proof` names it from outside.
pub(crate) mod proof {
    pub trait FoundAt<T, Index> {}

    pub trait SupersetOf<Req, Idxs> {}
}

/// Proof that table `T` appears somewhere in a scope list, found at
/// compile-time-inferred position `Index`. `Index` is never spelled out by
/// callers: it's inferred, exactly like frunk's `Plucker`. It is also what
/// keeps the two impls below structurally distinct rather than overlapping.
#[diagnostic::on_unimplemented(
    message = "`{T}` is not available in this query's scope",
    label = "add `.join(<table>, ..)` (or `.from(..)`) for `{T}` before referencing its columns here",
    note = "columns can only be referenced once their table has been joined into the current FROM/JOIN scope. In a generic helper, give each table its own `Idx` parameter, since one shared index matches no scope"
)]
pub trait Find<T: Table, Index>: proof::FoundAt<T, Index> {
    /// The nullability `T` has in this scope (derived from how it was
    /// joined, not asserted manually).
    type Nullability: Nullability;
}

impl<T: Table, N: Nullability, Tail> proof::FoundAt<T, Here> for Cons<TableSlot<T, N>, Tail> {}

impl<T: Table, N: Nullability, Tail> Find<T, Here> for Cons<TableSlot<T, N>, Tail> {
    type Nullability = N;
}

impl<T: Table, Head, Tail, I> proof::FoundAt<T, There<I>> for Cons<Head, Tail> where Tail: Find<T, I>
{}

#[diagnostic::do_not_recommend]
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
/// `Idxs` mirrors `Find`'s own `Index` parameter: it holds the per-element
/// lookup indices, and callers never name it explicitly. It has to live in
/// the trait's parameter list, because an index constrained only by the
/// `where` clause isn't accepted: Rust has no existential quantification
/// over impl generics.
#[diagnostic::on_unimplemented(
    message = "this expression references a table that isn't in scope here",
    label = "requires {Req}, but the current query scope doesn't contain all of it",
    note = "in a generic helper, `Idxs` has to be a type parameter of its own: one shared index matches no scope, however right the tables look"
)]
pub trait Superset<Req, Idxs>: proof::SupersetOf<Req, Idxs> {}

impl<S> proof::SupersetOf<Nil, Nil> for S {}
impl<S> Superset<Nil, Nil> for S {}

impl<S, Head: Table, Tail, IdxHead, IdxsTail>
    proof::SupersetOf<Cons<Head, Tail>, Cons<IdxHead, IdxsTail>> for S
where
    S: Find<Head, IdxHead> + Superset<Tail, IdxsTail>,
{
}

impl<S, Head: Table, Tail, IdxHead, IdxsTail> Superset<Cons<Head, Tail>, Cons<IdxHead, IdxsTail>>
    for S
where
    S: Find<Head, IdxHead> + Superset<Tail, IdxsTail>,
{
}

/// A scope's tables without their nullability: the `Req` list an expression
/// carries. `Select::correlated`'s `EXISTS` needs it to say "this condition
/// references everything the outer query had in scope", so a subquery can't
/// be filtered onto a query that never joined those tables.
pub trait ScopeTables {
    type Tables;
}

impl ScopeTables for Nil {
    type Tables = Nil;
}

impl<T: Table, N: Nullability, Tail: ScopeTables> ScopeTables for Cons<TableSlot<T, N>, Tail> {
    type Tables = Cons<T, Tail::Tables>;
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

pub(crate) mod wrap {
    /// Sealed with the nullability as a parameter, for the reason `proof`
    /// gives, because `Self` is the caller's own type wherever a schema
    /// declares one. An open impl lets a schema say a `LEFT JOIN` leaves its
    /// column NOT NULL, and the row then decodes a NULL into a non-`Option`.
    pub trait Sealed<N> {}
}

/// Idempotently wrap a SQL type as nullable-or-not depending on a
/// `Nullability` marker, without double-wrapping an already-`Nullable<T>`
/// column. Column accessors compose this with `Find::Nullability` so that
/// NULL-ability is *derived* from join shape rather than requiring a manual
/// `.nullable()` assertion (diesel's documented wart).
pub trait WrapNullable<N: Nullability>: wrap::Sealed<N> {
    type Output;
}

/// Wraps a base SQL type as nullable. A distinct outer shape from any base
/// type, so it can have its own `WrapNullable<MaybeNull>` impl without
/// overlapping the per-base-type impls below.
pub struct Nullable<T>(PhantomData<T>);

// A blanket impl is coherence-safe for `NotNull`, unlike for `MaybeNull`
// below: no second impl competes for the `NotNull` slot.
impl<T> wrap::Sealed<NotNull> for T {}

impl<T> WrapNullable<NotNull> for T {
    type Output = T;
}

impl<T> wrap::Sealed<MaybeNull> for Nullable<T> {}

impl<T> WrapNullable<MaybeNull> for Nullable<T> {
    type Output = Nullable<T>;
}

// No blanket `impl<T> WrapNullable<MaybeNull> for T` beside the
// `Nullable<T>` impl above: coherence treats a fully generic `T` as possibly
// covering `Nullable<_>`, so the two overlap. Each base SQL type gets its own
// non-generic impl instead, from `expr`'s `sql_leaf_type!`.
