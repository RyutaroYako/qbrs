//! `prepare!{}`: declares a named, typed parameter struct for a reusable
//! prepared query (`select::Prepared`). See that module's doc comment for
//! the design and its soundness caveat.

/// ```ignore
/// prepare! {
///     struct ByEmail { email: Text }
/// }
///
/// let query = select((users::id,))
///     .from::<Postgres, _>(users::Table)
///     .filter(users::email.eq(ByEmail::email()))
///     .prepare();
///
/// let (sql, params) = query.resolve(ByEmail { email: "a@example.com".into() })?;
/// ```
#[macro_export]
macro_rules! prepare {
    (struct $name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Default)]
        pub struct $name {
            $(pub $field: <$ty as $crate::expr::SqlType>::Native,)*
        }

        #[allow(dead_code, non_snake_case)]
        impl $name {
            $(
                pub fn $field() -> $crate::expr::Expr<$crate::scope::Nil, $ty> {
                    // Qualified by where it was declared: `Params` is a free
                    // parameter of `.prepare()`, so two `prepare!` structs
                    // sharing a field name would otherwise fill each other's
                    // placeholders with no complaint from anyone.
                    $crate::expr::placeholder::<$ty>(::std::concat!(
                        ::std::module_path!(), "::", ::std::stringify!($name),
                        ".", ::std::stringify!($field)
                    ))
                }
            )*
        }

        impl $crate::select::PreparedParams for $name {
            fn into_named_values(self) -> ::std::vec::Vec<(&'static str, $crate::expr::Value)> {
                ::std::vec![
                    $((
                        ::std::concat!(
                            ::std::module_path!(), "::", ::std::stringify!($name),
                            ".", ::std::stringify!($field)
                        ),
                        ::std::convert::Into::into(self.$field),
                    ),)*
                ]
            }
        }
    };
}
