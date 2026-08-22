//! What a `.select(..)` list decodes to once a row comes back.

use crate::expr::{AliasKey, Aliased, Column, ColumnKey, Expr, ExprKind, Keyed, SqlType};
use crate::render::SelectItem;
use crate::row::{Named, Row, RowCons, RowKey, RowNil};
use crate::scope::{Find, Nil, Superset, Table, WrapNullable};

/// One element of a selection list: the key its value is filed under in the
/// resulting `Row`, and the Rust type it decodes to.
///
/// Parameterized by `Scope` so a bare column's `Value` is `Option<T>` when —
/// and only when — that column's table is nullable in *this* query, via
/// `scope::Find::Nullability` + `WrapNullable`. Nullability is therefore
/// derived from join shape rather than asserted with a manual `.nullable()`.
///
/// Scope membership is proven as a side effect of this trait type-checking
/// at all, through the `Find`/`Superset` bounds below — so callers need no
/// separate check.
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

impl<K: AliasKey, Inner, Scope, Idx> RowField<Scope, Idx> for Aliased<K, Inner>
where
    Inner: RowField<Scope, Idx>,
{
    type Value = <Inner as RowField<Scope, Idx>>::Value;
    fn item(&self) -> SelectItem {
        SelectItem::labeled(self.inner.item().kind, <K as Named>::NAME)
    }
}

/// A whole `SELECT` list. A tuple decodes to a `row::Row` keyed by each
/// element's `RowField::Key`; a single un-tupled element decodes to its bare
/// value, since there is nothing to key it against.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a valid selection list here",
    label = "a selection is a column, an expression, or a tuple of up to 16 of them",
    note = "every element also has to be in scope: `.from(..)`/`.join(..)` the tables it names"
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
scalar_selection!(impl[C: ColumnKey] Column<C>);
scalar_selection!(impl[S: SqlType] Expr<Nil, S>);
scalar_selection!(impl[K, Req, S: SqlType] Keyed<K, Req, S>);
scalar_selection!(impl[K, Inner] Aliased<K, Inner>);

macro_rules! row_chain {
    ($n:ident $i:ident) => {
        RowCons<<$n as RowKey>::Key, <$n as RowField<Scope, $i>>::Value, RowNil>
    };
    ($n:ident $i:ident, $($rest:tt)*) => {
        RowCons<
            <$n as RowKey>::Key,
            <$n as RowField<Scope, $i>>::Value,
            row_chain!($($rest)*),
        >
    };
}

macro_rules! tuple_selection {
    ($($n:ident $i:ident),+) => {
        #[allow(non_snake_case)]
        impl<Scope, $($n,)+ $($i,)+> Selection<Scope, ($($i,)+)> for ($($n,)+)
        where
            $($n: RowField<Scope, $i>,)+
        {
            type Output = Row<row_chain!($($n $i),+)>;
            fn items(&self) -> Vec<SelectItem> {
                let ($($n,)+) = self;
                vec![$(RowField::item($n)),+]
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
