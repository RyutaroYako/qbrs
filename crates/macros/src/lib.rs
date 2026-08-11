//! `#[derive(Table)]`: turns a plain Rust struct (native field types —
//! `i64`, `String`, `Option<String>`, ...) into a schema module (`mod
//! users { pub struct Table; pub const id: Column<Table, BigInt> = ..; }`)
//! plus `*Insert`/`*Update` companion structs using the `Defaultable<T>`
//! design from the plan. Nullability is inferred from `Option<T>` wrapping
//! rather than a separate `not_null`/`nullable` attribute — one source of
//! truth, less ceremony. `#[column(primary_key)]`, `#[column(generated)]`,
//! and `#[column(default)]` are the only per-field attributes.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, Type, parse_macro_input};

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
    /// -> `BigInt`). Computed once and reused everywhere it's needed
    /// (schema module, and the typed-NULL binding in Insert/Update codegen)
    /// rather than re-deriving it from `base_ty` at each call site.
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
                    // `default = "expr"` is accepted but the expression
                    // itself is a migration/DDL concern (v2), not needed
                    // at the Rust-type level — just consume the value.
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
/// `SqlType` marker path. Deliberately a closed match, not a fallback —
/// an unsupported type should be a clear compile error naming the type,
/// not a confusing failure somewhere downstream.
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
    let mut consts = Vec::new();
    for c in columns {
        let name = &c.field_name;
        let base_sql_ty = &c.sql_type;
        // A nullable *schema* column must get `Column<Table, Nullable<X>>`,
        // not `Column<Table, X>` — this is independent of (and more basic
        // than) the separate, already-documented limitation that join-
        // derived nullability doesn't yet flow into `Selection::Output`.
        // Missing this wrapper here was a real bug caught by the Postgres
        // integration test: `.eq()` on a nullable column would otherwise
        // demand a non-`Option` value, and decoded rows would reject an
        // actual NULL instead of yielding `None`.
        let col_sql_ty = if c.nullable {
            quote! { ::qbrs::scope::Nullable<#base_sql_ty> }
        } else {
            quote! { #base_sql_ty }
        };
        let col_name_str = name.to_string();
        consts.push(quote! {
            #[allow(non_upper_case_globals)]
            pub const #name: ::qbrs::expr::Column<Table, #col_sql_ty> =
                ::qbrs::expr::Column::new(#col_name_str);
        });
    }

    Ok(quote! {
        #[allow(non_snake_case)]
        pub mod #mod_ident {
            pub struct Table;
            impl ::qbrs::scope::Table for Table {
                const NAME: &'static str = #table_name;
            }
            #(#consts)*
        }
    })
}

/// Per the plan's rule table:
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
/// "untouched" from "explicit NULL" (see `update::UpdateRow`). Primary
/// keys and generated columns are excluded — updating either is not a
/// supported v1 operation.
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
