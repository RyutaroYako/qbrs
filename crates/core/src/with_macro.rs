//! `with!{}`: declares a named pseudo-table for a common table expression —
//! see `cte`'s module doc comment for the full design.

/// ```ignore
/// with! {
///     struct recent_orders { id: qbrs::expr::Integer, total: qbrs::expr::BigInt }
/// }
///
/// let recent = select((orders::id, orders::total))
///     .from::<Postgres, _>(orders::Table)
///     .filter(orders::total.gt(1000));
///
/// let rows = select((recent_orders::id, recent_orders::total))
///     .with(cte::with(recent_orders::Table, &recent))
///     .from::<Postgres, _>(recent_orders::Table)
///     .to_sql();
/// ```
///
/// Generates the same shape `#[derive(Table)]` does (a `Table` marker plus
/// one `Column<Table, S>` const per field) so the resulting pseudo-table is
/// usable anywhere a real one is — plus a `CteShape` impl recording the
/// declared columns' native types as a tuple, which `cte::with` checks
/// against the actual query passed to it.
///
/// Column types must be written as a full path (`qbrs::expr::Integer`, not a
/// bare `Integer` even with a `use qbrs::expr::Integer` already in scope) —
/// unlike `prepare!{}`'s fields, these end up nested inside a generated
/// `mod $name { .. }` (needed for the `name::column` access pattern to
/// match real tables), and module boundaries aren't transparent to a
/// surrounding `use` the way hygiene makes local bindings transparent.
#[macro_export]
macro_rules! with {
    (struct $name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[allow(non_snake_case)]
        pub mod $name {
            pub struct Table;
            impl $crate::scope::Table for Table {
                const NAME: &'static str = stringify!($name);
            }
            $(
                #[allow(non_upper_case_globals)]
                pub const $field: $crate::expr::Column<Table, $ty> =
                    $crate::expr::Column::new(stringify!($field));
            )*

            impl $crate::cte::CteShape for Table {
                type Shape = ($(<$ty as $crate::expr::SqlType>::Native,)*);
                const COLUMN_NAMES: &'static [&'static str] = &[$(stringify!($field)),*];
            }
        }
    };
}
