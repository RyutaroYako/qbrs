//! `#[derive(Table)]`: turns a struct of native field types into a schema
//! module (`mod users { pub struct Table; pub mod columns { pub struct id; }
//! pub const id: Column<columns::id> = ..; }`) plus `*Insert`/`*Update`
//! companion structs. Nullability is inferred from `Option<T>` wrapping
//! rather than a separate attribute. `#[column(primary_key)]`,
//! `#[column(generated)]`, and `#[column(default)]` are the only per-field
//! attributes.
//!
//! `label!` declares output-column names for computed selections, and
//! `#[derive(FromRow)]` maps a row into a plain struct by field name. All
//! three live here rather than as `macro_rules!` in `qbrs-core` (where
//! `sql!`, `prepare!`, and `with!` live) because all three turn an
//! identifier into something a declarative macro cannot produce: another
//! identifier (`HasEmail` from `email`), or its type-level spelling.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{Data, DeriveInput, Fields, Ident, Token, Type, parse_macro_input};

#[proc_macro_derive(Table, attributes(table, column))]
pub fn derive_table(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

struct ColumnInfo {
    field_name: Ident,
    /// The inner type with any `Option<..>` wrapper stripped off.
    base_ty: Type,
    /// The `qbrs::expr` `SqlType` marker mapped from `base_ty` (e.g. `i64`
    /// -> `BigInt`).
    sql_type: TokenStream2,
    nullable: bool,
    primary_key: bool,
    generated: bool,
    has_default: bool,
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let struct_ident = &input.ident;
    let table_name =
        table_name_attr(&input)?.unwrap_or_else(|| to_snake_case(&struct_ident.to_string()));
    let mod_ident = format_ident!("{}", to_snake_case(&struct_ident.to_string()));

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    struct_ident,
                    "#[derive(Table)] requires named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                struct_ident,
                "#[derive(Table)] only supports structs",
            ));
        }
    };

    let mut columns = Vec::new();
    for f in fields {
        let field_name = f.ident.clone().expect("named field");
        let (nullable, base_ty) = strip_option(&f.ty);
        let mut primary_key = false;
        let mut generated = false;
        let mut has_default = false;
        for attr in &f.attrs {
            if !attr.path().is_ident("column") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("primary_key") {
                    primary_key = true;
                } else if meta.path.is_ident("generated") {
                    generated = true;
                } else if meta.path.is_ident("default") {
                    has_default = true;
                    // `default = "expr"` parses, but the expression is a
                    // migration/DDL concern; nothing here needs its value.
                    if meta.input.peek(syn::Token![=]) {
                        let _: syn::Expr = meta.value()?.parse()?;
                    }
                } else {
                    return Err(meta.error("unknown #[column(..)] option"));
                }
                Ok(())
            })?;
        }
        // A primary key is always NOT NULL by definition, regardless of
        // how the field happens to be typed.
        let nullable = nullable && !primary_key;
        let sql_type = sql_type_for(&base_ty)?;
        columns.push(ColumnInfo {
            field_name,
            base_ty,
            sql_type,
            nullable,
            primary_key,
            generated,
            has_default,
        });
    }

    let schema_mod = gen_schema_mod(&mod_ident, &table_name, &columns)?;
    let insert_struct = gen_insert_struct(struct_ident, &mod_ident, &columns);
    let update_struct = gen_update_struct(struct_ident, &mod_ident, &columns);

    Ok(quote! {
        #schema_mod
        #insert_struct
        #update_struct
    })
}

fn table_name_attr(input: &DeriveInput) -> syn::Result<Option<String>> {
    for attr in &input.attrs {
        if !attr.path().is_ident("table") {
            continue;
        }
        let mut name = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let value: syn::LitStr = meta.value()?.parse()?;
                name = Some(value.value());
                Ok(())
            } else {
                Err(meta.error("unknown #[table(..)] option, expected `name = \"...\"`"))
            }
        })?;
        return Ok(name);
    }
    Ok(None)
}

/// `Option<T>` -> `(true, T)`; anything else -> `(false, original type)`.
fn strip_option(ty: &Type) -> (bool, Type) {
    if let Type::Path(p) = ty
        && let Some(seg) = p.path.segments.last()
        && seg.ident == "Option"
        && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return (true, inner.clone());
    }
    (false, ty.clone())
}

/// Maps a base (non-`Option`) native Rust type to its `qbrs_core::expr`
/// `SqlType` marker path. A closed match, so an unsupported type is a clear
/// compile error naming the type rather than a downstream failure.
fn sql_type_for(ty: &Type) -> syn::Result<TokenStream2> {
    if let Type::Path(p) = ty
        && let Some(seg) = p.path.segments.last()
    {
        let name = seg.ident.to_string();
        let path = match name.as_str() {
            "i32" => quote! { ::qbrs::expr::Integer },
            "i64" => quote! { ::qbrs::expr::BigInt },
            "f64" => quote! { ::qbrs::expr::Real },
            "String" => quote! { ::qbrs::expr::Text },
            "bool" => quote! { ::qbrs::expr::Bool },
            "Vec" => quote! { ::qbrs::expr::Bytes },
            other => {
                return Err(syn::Error::new_spanned(
                    ty,
                    format!(
                        "unsupported column type `{other}` — supported: i32, i64, f64, String, bool, Vec<u8>, or Option<..> of one of those"
                    ),
                ));
            }
        };
        return Ok(path);
    }
    Err(syn::Error::new_spanned(ty, "unsupported column type"))
}

fn gen_schema_mod(
    mod_ident: &Ident,
    table_name: &str,
    columns: &[ColumnInfo],
) -> syn::Result<TokenStream2> {
    let mut keys = Vec::new();
    let mut consts = Vec::new();
    let mut accessors = Vec::new();
    let mut accessor_uses = Vec::new();
    for c in columns {
        let name = &c.field_name;
        let base_sql_ty = &c.sql_type;
        // A nullable schema column needs `Nullable<X>`: without the wrapper,
        // `.eq()` would demand a non-`Option` value and decoding a real NULL
        // would fail instead of yielding `None`.
        let col_sql_ty = if c.nullable {
            quote! { ::qbrs::scope::Nullable<#base_sql_ty> }
        } else {
            quote! { #base_sql_ty }
        };
        let col_name_str = name.to_string();
        let type_name = type_level_name(&col_name_str);
        keys.push(quote! {
            #[derive(Clone, Copy)]
            pub struct #name;
            impl ::qbrs::expr::ColumnKey for #name {
                type Table = super::Table;
                type Sql = #col_sql_ty;
                const NAME: &'static str = #col_name_str;
            }
            impl ::qbrs::row::Named for #name {
                type Name = #type_name;
            }
        });
        consts.push(quote! {
            #[allow(non_upper_case_globals)]
            pub const #name: ::qbrs::expr::Column<columns::#name> =
                ::qbrs::expr::Column::new();
        });
        let trait_ident = format_ident!("Has{}", to_camel_case(&col_name_str));
        accessors.push(accessor_trait(
            &trait_ident,
            name,
            &quote! { columns::#name },
        ));
        accessor_uses.push(quote! {
            #[allow(unused_imports)]
            pub use #mod_ident::#trait_ident as _;
        });
    }

    Ok(quote! {
        #[allow(non_snake_case)]
        pub mod #mod_ident {
            pub struct Table;
            impl ::qbrs::scope::Table for Table {
                const NAME: &'static str = #table_name;
            }

            #[allow(non_camel_case_types)]
            pub mod columns {
                #(#keys)*
            }

            #(#consts)*
            #(#accessors)*
        }

        #(#accessor_uses)*
    })
}

/// One column's `row.<name>()` accessor. The `Idx` parameter is the same
/// inferred lookup index `row::GetField` and `scope::Find` carry; it can't
/// be hidden, since an impl generic constrained only by a `where` clause
/// isn't accepted.
fn accessor_trait(trait_ident: &Ident, method: &Ident, key: &TokenStream2) -> TokenStream2 {
    quote! {
        pub trait #trait_ident<Idx> {
            type Value;
            fn #method(&self) -> &Self::Value;
        }

        impl<L, Idx> #trait_ident<Idx> for ::qbrs::row::Row<L>
        where
            L: ::qbrs::row::GetField<#key, Idx>,
        {
            type Value = <L as ::qbrs::row::GetField<#key, Idx>>::Value;
            fn #method(&self) -> &Self::Value {
                self.get(self::#method)
            }
        }
    }
}

/// `#[derive(FromRow)]`: fills the struct from a `Row` by matching each
/// field's name against the row's keys. The struct itself stays free of
/// column paths and query shape — the only thing it declares is what it
/// wants called what.
#[proc_macro_derive(FromRow)]
pub fn derive_from_row(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_from_row(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn expand_from_row(input: DeriveInput) -> syn::Result<TokenStream2> {
    let struct_ident = &input.ident;
    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    struct_ident,
                    "#[derive(FromRow)] requires named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                struct_ident,
                "#[derive(FromRow)] only supports structs",
            ));
        }
    };
    if fields.is_empty() {
        return Err(syn::Error::new_spanned(
            struct_ident,
            "#[derive(FromRow)] needs at least one field to fill",
        ));
    }

    // Field markers live in their own module so a failed lookup reports
    // `user_summary_fields::email` rather than the type-level spelling.
    let fields_mod = format_ident!("{}_fields", to_snake_case(&struct_ident.to_string()));

    let mut markers = Vec::new();
    let mut idx_params = Vec::new();
    let mut bounds = Vec::new();
    let mut steps = Vec::new();
    let mut inits = Vec::new();
    let mut receiver = quote! { L };

    for (position, f) in fields.iter().enumerate() {
        let field_name = f.ident.clone().expect("named field");
        let field_ty = &f.ty;
        let type_name = type_level_name(&field_name.to_string());
        markers.push(quote! {
            pub struct #field_name;
            impl ::qbrs::row::Named for #field_name {
                type Name = #type_name;
            }
        });

        let idx = format_ident!("Idx{position}");
        let marker = quote! { #fields_mod::#field_name };
        bounds.push(quote! {
            #receiver: ::qbrs::row::TakeNamed<#marker, #idx, Value = #field_ty>
        });
        receiver = quote! { <#receiver as ::qbrs::row::TakeNamed<#marker, #idx>>::Rest };

        // Bindings are numbered rather than named after the field: a field
        // name can also be a unit struct in scope, which a bare identifier
        // pattern would resolve to instead of introducing a binding.
        let binding = format_ident!("__field{position}");
        steps.push(quote! {
            let (#binding, row) = row.take_named::<#marker, #idx>();
        });
        inits.push(quote! { #field_name: #binding });
        idx_params.push(idx);
    }

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        mod #fields_mod {
            #(#markers)*
        }

        impl<L, #(#idx_params),*> ::qbrs::row::FromRow<L, (#(#idx_params,)*)> for #struct_ident
        where
            #(#bounds,)*
        {
            fn from_row(row: ::qbrs::row::Row<L>) -> Self {
                #(#steps)*
                let _ = row;
                Self { #(#inits),* }
            }
        }
    })
}

/// `label!(rank_in_user, rank_overall);` — declares output-column names for
/// computed selections, in a `label` module so they can never be shadowed by
/// a local binding of the same name. One invocation per scope; declaring it
/// inside the function that runs the query keeps the names next to their use
/// and sidesteps that limit.
#[proc_macro]
pub fn label(input: TokenStream) -> TokenStream {
    let names = parse_macro_input!(input with Punctuated::<Ident, Token![,]>::parse_terminated);
    let mut decls = Vec::new();
    let mut uses = Vec::new();
    for name in &names {
        let name_str = name.to_string();
        let type_name = type_level_name(&name_str);
        let trait_ident = format_ident!("Has{}", to_camel_case(&name_str));
        let accessor = accessor_trait(&trait_ident, name, &quote! { #name });
        decls.push(quote! {
            #[allow(non_camel_case_types)]
            #[derive(Clone, Copy)]
            pub struct #name;
            impl ::qbrs::row::RowKey for #name {
                type Key = #name;
            }
            impl ::qbrs::row::AliasKey for #name {
                const NAME: &'static str = #name_str;
            }
            impl ::qbrs::row::Named for #name {
                type Name = #type_name;
            }
            #accessor
        });
        uses.push(quote! {
            #[allow(unused_imports)]
            pub use label::#trait_ident as _;
        });
    }
    quote! {
        pub mod label {
            #(#decls)*
        }
        #(#uses)*
    }
    .into()
}

/// An identifier's type-level spelling, one `char` per cell — the bridge
/// that lets a `#[derive(FromRow)]` field find a column it has never been
/// told the path of.
fn type_level_name(name: &str) -> TokenStream2 {
    let chars = name.chars();
    quote! { ::qbrs::type_name!(#(#chars),*) }
}

fn to_camel_case(s: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in s.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Field shape per column:
/// generated              -> excluded entirely
/// not_null, no default   -> `T` (required, taken by `new()`)
/// not_null, has default  -> `Defaultable<T>`
/// nullable, no default   -> `Option<T>`
/// nullable, has default  -> `Defaultable<Option<T>>`
fn gen_insert_struct(
    struct_ident: &Ident,
    mod_ident: &Ident,
    columns: &[ColumnInfo],
) -> TokenStream2 {
    let insert_ident = format_ident!("{}Insert", struct_ident);
    let insertable: Vec<&ColumnInfo> = columns.iter().filter(|c| !c.generated).collect();

    let fields = insertable.iter().map(|c| {
        let name = &c.field_name;
        let base = &c.base_ty;
        let ty = match (c.nullable, c.has_default) {
            (false, false) => quote! { #base },
            (false, true) => quote! { ::qbrs::insert::Defaultable<#base> },
            (true, false) => quote! { ::std::option::Option<#base> },
            (true, true) => quote! { ::qbrs::insert::Defaultable<::std::option::Option<#base>> },
        };
        quote! { pub #name: #ty }
    });

    let required: Vec<&&ColumnInfo> = insertable
        .iter()
        .filter(|c| !c.nullable && !c.has_default)
        .collect();
    let new_params = required.iter().map(|c| {
        let name = &c.field_name;
        let base = &c.base_ty;
        quote! { #name: impl ::std::convert::Into<#base> }
    });
    let new_assigns = insertable.iter().map(|c| {
        let name = &c.field_name;
        if !c.nullable && !c.has_default {
            quote! { #name: #name.into() }
        } else if c.nullable && !c.has_default {
            quote! { #name: ::std::option::Option::None }
        } else {
            quote! { #name: ::std::default::Default::default() }
        }
    });

    let setters = insertable.iter().filter(|c| c.nullable || c.has_default).map(|c| {
        let name = &c.field_name;
        let base = &c.base_ty;
        if c.nullable && !c.has_default {
            quote! {
                pub fn #name(mut self, value: impl ::std::convert::Into<#base>) -> Self {
                    self.#name = ::std::option::Option::Some(value.into());
                    self
                }
            }
        } else if !c.nullable && c.has_default {
            quote! {
                pub fn #name(mut self, value: impl ::std::convert::Into<#base>) -> Self {
                    self.#name = ::qbrs::insert::Defaultable::Value(value.into());
                    self
                }
            }
        } else {
            quote! {
                pub fn #name(mut self, value: impl ::std::convert::Into<#base>) -> Self {
                    self.#name = ::qbrs::insert::Defaultable::Value(::std::option::Option::Some(value.into()));
                    self
                }
            }
        }
    });

    let columns_arr = insertable.iter().map(|c| c.field_name.to_string());
    let into_values = insertable.iter().map(|c| {
        let name = &c.field_name;
        let sql_ty = &c.sql_type;
        match (c.nullable, c.has_default) {
            (false, false) => quote! {
                ::qbrs::insert::InsertValue::Value(::std::convert::Into::into(self.#name))
            },
            (false, true) => quote! {
                ::std::convert::Into::<::qbrs::insert::InsertValue>::into(self.#name)
            },
            (true, false) => quote! {
                match self.#name {
                    ::std::option::Option::Some(v) => ::qbrs::insert::InsertValue::Value(::std::convert::Into::into(v)),
                    ::std::option::Option::None => ::qbrs::insert::InsertValue::Value(<#sql_ty as ::qbrs::expr::NullValue>::NULL_VALUE),
                }
            },
            (true, true) => quote! {
                match self.#name {
                    ::qbrs::insert::Defaultable::Default => ::qbrs::insert::InsertValue::Default,
                    ::qbrs::insert::Defaultable::Value(::std::option::Option::Some(v)) => ::qbrs::insert::InsertValue::Value(::std::convert::Into::into(v)),
                    ::qbrs::insert::Defaultable::Value(::std::option::Option::None) => ::qbrs::insert::InsertValue::Value(<#sql_ty as ::qbrs::expr::NullValue>::NULL_VALUE),
                }
            },
        }
    });

    quote! {
        pub struct #insert_ident {
            #(#fields,)*
        }

        impl #insert_ident {
            pub fn new(#(#new_params),*) -> Self {
                Self {
                    #(#new_assigns,)*
                }
            }

            #(#setters)*
        }

        impl ::qbrs::insert::InsertRow for #insert_ident {
            type Table = #mod_ident::Table;
            const COLUMNS: &'static [&'static str] = &[#(#columns_arr),*];
            fn into_values(self) -> ::std::vec::Vec<::qbrs::insert::InsertValue> {
                ::std::vec![#(#into_values),*]
            }
        }
    }
}

/// Update struct: every field optional (untouched vs. touched); nullable
/// columns get a doubly-nested `Option<Option<T>>` to distinguish
/// "untouched" from "explicit NULL". Primary keys and generated columns are
/// excluded; updating either isn't supported.
fn gen_update_struct(
    struct_ident: &Ident,
    mod_ident: &Ident,
    columns: &[ColumnInfo],
) -> TokenStream2 {
    let update_ident = format_ident!("{}Update", struct_ident);
    let updatable: Vec<&ColumnInfo> = columns
        .iter()
        .filter(|c| !c.generated && !c.primary_key)
        .collect();

    let fields = updatable.iter().map(|c| {
        let name = &c.field_name;
        let base = &c.base_ty;
        let ty = if c.nullable {
            quote! { ::std::option::Option<::std::option::Option<#base>> }
        } else {
            quote! { ::std::option::Option<#base> }
        };
        quote! { pub #name: #ty }
    });

    let sets = updatable.iter().map(|c| {
        let name = &c.field_name;
        let col_name_str = c.field_name.to_string();
        let sql_ty = &c.sql_type;
        if c.nullable {
            quote! {
                if let ::std::option::Option::Some(v) = self.#name {
                    sets.push((#col_name_str, match v {
                        ::std::option::Option::Some(inner) => ::std::convert::Into::into(inner),
                        ::std::option::Option::None => <#sql_ty as ::qbrs::expr::NullValue>::NULL_VALUE,
                    }));
                }
            }
        } else {
            quote! {
                if let ::std::option::Option::Some(v) = self.#name {
                    sets.push((#col_name_str, ::std::convert::Into::into(v)));
                }
            }
        }
    });

    quote! {
        #[derive(::std::default::Default)]
        pub struct #update_ident {
            #(#fields,)*
        }

        impl ::qbrs::update::UpdateRow for #update_ident {
            type Table = #mod_ident::Table;
            fn sets(self) -> ::std::vec::Vec<(&'static str, ::qbrs::expr::Value)> {
                let mut sets = ::std::vec::Vec::new();
                #(#sets)*
                sets
            }
        }
    }
}

fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
