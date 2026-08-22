//! Synthetic schema plus escalating join-count scope chains, measuring
//! whether the flat cons-list `Find<T, Idx>` design keeps `cargo check`
//! linear as join count grows — diesel's join-tree is documented to blow up
//! exponentially past ~7 joins (diesel#3223).
//!
//! `cols_*` binaries measure the other axis: `row::GetField` walks a
//! selection's key list the same way `Find` walks a scope, so selection
//! width has its own linear cost that join count doesn't cover.

use qbrs_core::scope::{MapNullable, Superset, Table};

macro_rules! declare_tables {
    ($($name:ident),* $(,)?) => {
        $(
            pub struct $name;
            impl Table for $name {
                const NAME: &'static str = stringify!($name);
            }
            impl qbrs_core::scope::BaseTable for $name {}
        )*
    };
}

// 100 synthetic tables: headroom to stress-test well past diesel's ~7-join
// blowup point, including a full-superset worst case where every table is
// required at once.
declare_tables!(
    T00, T01, T02, T03, T04, T05, T06, T07, T08, T09, T10, T11, T12, T13, T14, T15, T16, T17, T18,
    T19, T20, T21, T22, T23, T24, T25, T26, T27, T28, T29, T30, T31, T32, T33, T34, T35, T36, T37,
    T38, T39, T40, T41, T42, T43, T44, T45, T46, T47, T48, T49, T50, T51, T52, T53, T54, T55, T56,
    T57, T58, T59, T60, T61, T62, T63, T64, T65, T66, T67, T68, T69, T70, T71, T72, T73, T74, T75,
    T76, T77, T78, T79, T80, T81, T82, T83, T84, T85, T86, T87, T88, T89, T90, T91, T92, T93, T94,
    T95, T96, T97, T98, T99,
);

/// Build a nested `Cons<TableSlot<Head, NotNull>, ...>` scope type from a
/// list of table types, innermost (first FROM'd table) last.
#[macro_export]
macro_rules! scope_of {
    () => { $crate::__private::Nil };
    ($head:ty $(, $tail:ty)* $(,)?) => {
        $crate::__private::Cons<$crate::__private::TableSlot<$head, $crate::__private::NotNull>, scope_of!($($tail),*)>
    };
}

/// Build a bare `Cons<Head, ...>` list of table types with no `TableSlot`
/// wrapper: this is what `Superset`'s `Req` expects, since "which tables
/// does this expression touch" carries no nullability. Passing a
/// `scope_of!` list here instead fails `Superset`'s `Head: Table` bound and
/// reports as a missing join.
#[macro_export]
macro_rules! req_of {
    () => { $crate::__private::Nil };
    ($head:ty $(, $tail:ty)* $(,)?) => {
        $crate::__private::Cons<$head, req_of!($($tail),*)>
    };
}

/// Declares `n` columns on table `$table`: a `ColumnKey` marker per column
/// plus the `Column` const that selects it, exactly what
/// `#[derive(Table)]` emits minus the accessor traits.
#[macro_export]
macro_rules! declare_columns {
    ($table:ty, $($name:ident),* $(,)?) => {
        #[allow(non_camel_case_types)]
        pub mod columns {
            use super::*;
            $(
                #[derive(Clone, Copy)]
                pub struct $name;
                impl $crate::__private::ColumnKey for $name {
                    type Table = $table;
                    type Sql = $crate::__private::BigInt;
                }
                impl $crate::__private::Named for $name {
                    type Name = $crate::__private::NameEnd;
                    const NAME: &'static str = stringify!($name);
                }
            )*
        }

        $(
            #[allow(non_upper_case_globals)]
            pub const $name: $crate::__private::Column<columns::$name> =
                $crate::__private::Column::new();
        )*
    };
}

/// Build a `Row<RowCons<..>>` type from a list of column keys, so the
/// `GetField` walk can be measured past the arity a tuple selection allows.
#[macro_export]
macro_rules! row_of {
    () => { $crate::__private::RowNil };
    ($head:ty $(, $tail:ty)* $(,)?) => {
        $crate::__private::RowCons<$head, i64, row_of!($($tail),*)>
    };
}

// Re-exported under a stable path so the macros above resolve these types
// regardless of what the calling crate imported.
#[doc(hidden)]
pub mod __private {
    pub use qbrs_core::expr::{BigInt, Column, ColumnKey};
    pub use qbrs_core::row::{NameEnd, Named, RowCons, RowNil};
    pub use qbrs_core::scope::{Cons, Nil, NotNull, TableSlot};
}

/// Generic proof helper: "does `S` contain `T`". Exercises `Find` without
/// needing the caller to name the inferred `Idx`.
pub fn assert_contains<S, T: Table, I>()
where
    S: qbrs_core::scope::Find<T, I>,
{
}

/// Exercises `Superset<Req, _>` for a multi-table requirement list.
pub fn assert_superset<S, Req, Idxs>()
where
    S: Superset<Req, Idxs>,
{
}

/// Exercises `MapNullable` (the RIGHT/FULL JOIN nullability-flip operation)
/// over a scope of arbitrary depth.
pub fn assert_map_nullable<S: MapNullable>() {}

/// Exercises `row::GetField<K, _>` at whatever depth `L` puts `K` at — the
/// selection-width counterpart to `assert_contains`.
pub fn assert_field<L, K, I>()
where
    L: qbrs_core::row::Field<K, I>,
{
}
