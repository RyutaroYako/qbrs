//! What a `.select(..)` list decodes to once a row comes back.

use crate::expr::{Column, Expr, ExprKind, SqlType};
use crate::scope::{Find, Superset, Table, WrapNullable};

/// Given the query's `Scope`. Parameterized by `Scope` (rather than just
/// being `Selection` with no type parameter) specifically so a bare
/// column's `Output` can be `Option<T>` when — and only when — that
/// column's table is nullable *in this particular query's scope* (via a
/// LEFT/RIGHT/FULL join), derived through `scope::Find::Nullability` +
/// `WrapNullable` exactly the way `Select`'s own join methods derive it.
/// This is the direct fix for diesel's documented wart (nullability from
/// joins must be asserted manually with `.nullable()`) — here it's
/// structurally derived, so a query that outer-joins a table and selects
/// its columns decodes them as `Option<T>` with no action from the caller.
///
/// `Expr<Req, S>` (an arbitrary computed/raw expression, as opposed to a
/// bare column) does *not* get this treatment — an expression's `ExprKind`
/// is opaque, so there's no join-derived nullability to look up for it;
/// its `Output` is always just `S::Native`, matching what the caller
/// declared when building it (e.g. via `sql!(Nullable<Text>, "...")` for an
/// expression that can itself be NULL).
///
/// `Idx` mirrors `Find`'s own index parameter — see `scope::Superset`'s
/// doc comment for why an index used only in a `where` clause (rather than
/// in the trait's own parameter list) doesn't compile (E0207). A column's
/// membership in `Scope` is proven as a side effect of `Selection` itself
/// (via `Column`'s `Scope: Find<T, Idx>` bound, and `Expr`'s
/// `Scope: Superset<Req, Idx>` bound below) — if `Selection` type-checks at
/// all, everything selected is already proven in scope, with no separate
/// check needed at the call site.
pub trait Selection<Scope, Idx> {
    type Output;
    fn exprs(&self) -> Vec<ExprKind>;
}

impl<T: Table, S: SqlType, Scope, Idx> Selection<Scope, Idx> for Column<T, S>
where
    Scope: Find<T, Idx>,
    S: WrapNullable<<Scope as Find<T, Idx>>::Nullability>,
    <S as WrapNullable<<Scope as Find<T, Idx>>::Nullability>>::Output: SqlType,
{
    type Output =
        <<S as WrapNullable<<Scope as Find<T, Idx>>::Nullability>>::Output as SqlType>::Native;
    fn exprs(&self) -> Vec<ExprKind> {
        vec![ExprKind::Column {
            table: T::NAME,
            name: self.name,
        }]
    }
}

// `Idx` here is `Superset`'s own index list, not a `Find` lookup (an
// expression has no single table to look up) — checking `Scope: Superset<Req,
// Idx>` is what makes selecting an expression that references an
// out-of-scope table (e.g. `row_number().over(window().partition_by(..))`
// where the partitioned-by column's table was never joined) a compile error,
// not a silent gap. This used to be a fixed, concrete `Idx = ()` with no
// `Superset` bound at all, on the reasoning that `sql!{}`'s `Req` is always
// `Nil` (exempt from scope checking by design) so there was "nothing to
// check" — true for `sql!{}` specifically, but not for every `Expr`: window
// functions (see `window::Window`) build a real, non-`Nil` `Req` via
// `Concat` across `.partition_by()`/`.order_by()` calls, and that Req *does*
// need checking once the expression is selected. Fixing `Idx` to `Superset`'s
// own inferred index list (rather than reintroducing an unconstrained
// parameter) is sound for the same reason `Superset` itself is: for a
// concrete `Scope`/`Req`, `Find`'s structural indices give at most one valid
// decomposition, so nothing is actually ambiguous here — the old `()`-fixing
// avoided a *different*, hypothetical problem (an `Idx` with no constraining
// bound at all) that doesn't apply once `Idx` is tied to `Superset`.
impl<Req, S: SqlType, Scope, Idx> Selection<Scope, Idx> for Expr<Req, S>
where
    Scope: Superset<Req, Idx>,
{
    type Output = S::Native;
    fn exprs(&self) -> Vec<ExprKind> {
        vec![self.kind.clone()]
    }
}

impl<Scope, Idx, A: Selection<Scope, Idx>> Selection<Scope, Idx> for (A,) {
    type Output = (A::Output,);
    fn exprs(&self) -> Vec<ExprKind> {
        self.0.exprs()
    }
}

impl<Scope, IdxA, IdxB, A: Selection<Scope, IdxA>, B: Selection<Scope, IdxB>>
    Selection<Scope, (IdxA, IdxB)> for (A, B)
{
    type Output = (A::Output, B::Output);
    fn exprs(&self) -> Vec<ExprKind> {
        let (a, b) = self;
        let mut v = a.exprs();
        v.extend(b.exprs());
        v
    }
}

impl<
    Scope,
    IdxA,
    IdxB,
    IdxC,
    A: Selection<Scope, IdxA>,
    B: Selection<Scope, IdxB>,
    C: Selection<Scope, IdxC>,
> Selection<Scope, (IdxA, IdxB, IdxC)> for (A, B, C)
{
    type Output = (A::Output, B::Output, C::Output);
    fn exprs(&self) -> Vec<ExprKind> {
        let (a, b, c) = self;
        let mut v = a.exprs();
        v.extend(b.exprs());
        v.extend(c.exprs());
        v
    }
}

impl<
    Scope,
    IdxA,
    IdxB,
    IdxC,
    IdxD,
    A: Selection<Scope, IdxA>,
    B: Selection<Scope, IdxB>,
    C: Selection<Scope, IdxC>,
    D: Selection<Scope, IdxD>,
> Selection<Scope, (IdxA, IdxB, IdxC, IdxD)> for (A, B, C, D)
{
    type Output = (A::Output, B::Output, C::Output, D::Output);
    fn exprs(&self) -> Vec<ExprKind> {
        let (a, b, c, d) = self;
        let mut v = a.exprs();
        v.extend(b.exprs());
        v.extend(c.exprs());
        v.extend(d.exprs());
        v
    }
}

impl<
    Scope,
    IdxA,
    IdxB,
    IdxC,
    IdxD,
    IdxE,
    A: Selection<Scope, IdxA>,
    B: Selection<Scope, IdxB>,
    C: Selection<Scope, IdxC>,
    D: Selection<Scope, IdxD>,
    E: Selection<Scope, IdxE>,
> Selection<Scope, (IdxA, IdxB, IdxC, IdxD, IdxE)> for (A, B, C, D, E)
{
    type Output = (A::Output, B::Output, C::Output, D::Output, E::Output);
    fn exprs(&self) -> Vec<ExprKind> {
        let (a, b, c, d, e) = self;
        let mut v = a.exprs();
        v.extend(b.exprs());
        v.extend(c.exprs());
        v.extend(d.exprs());
        v.extend(e.exprs());
        v
    }
}
