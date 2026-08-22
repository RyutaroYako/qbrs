//! `#[derive(Table)]`: turns a struct of native field types into a schema
//! module (`mod users { pub struct Table; pub mod columns { pub struct id; }
//! pub const id: Column<columns::id> = ..; }`) plus `*Insert`/`*Update`
//! companion structs. Nullability is inferred from `Option<T>` wrapping
//! rather than a separate attribute. `#[column(primary_key)]`,
//! `#[column(generated)]`, and `#[column(default)]` are the only per-field
//! attributes.
//!
//! `with!` declares a CTE's pseudo-table, `label!` declares output-column
//! names for computed selections, and `#[derive(FromRow)]` maps a row into a
//! plain struct by field name. All four live here rather than as
//! `macro_rules!` in `qbrs-core` (where `sql!` and `prepare!` live) because
//! all four turn an identifier into something a declarative macro cannot
//! produce: another identifier (`HasEmail` from `email`), or its type-level
//! spelling.

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
        // A generic type is only the type it looks like when its argument
        // agrees: `Vec<u8>` is `bytea`, `Vec<String>` is nothing this crate
        // has, and saying so here is what keeps the match closed.
        let argument_ok = match name.as_str() {
            "Vec" => generic_argument_is(seg, "u8"),
            "DateTime" => generic_argument_is(seg, "Utc"),
            _ => true,
        };
        let path = match name.as_str() {
            _ if !argument_ok => {
                return Err(syn::Error::new_spanned(
                    ty,
                    format!(
                        "unsupported column type — `{name}` is a column type only as `Vec<u8>` \
                         (bytes) or `DateTime<Utc>` (timestamptz)"
                    ),
                ));
            }
            "i32" => quote! { ::qbrs::expr::Integer },
            "i64" => quote! { ::qbrs::expr::BigInt },
            "f64" => quote! { ::qbrs::expr::Real },
            "String" => quote! { ::qbrs::expr::Text },
            "bool" => quote! { ::qbrs::expr::Bool },
            "Vec" => quote! { ::qbrs::expr::Bytes },
            // Behind a feature in `qbrs-core`; naming one here without that
            // feature is an unresolved-path error at the marker, which says
            // which feature is missing better than this match could.
            "DateTime" => quote! { ::qbrs::expr::Timestamptz },
            "NaiveDate" => quote! { ::qbrs::expr::Date },
            "Uuid" => quote! { ::qbrs::expr::Uuid },
            "Decimal" => quote! { ::qbrs::expr::Numeric },
            other => {
                return Err(syn::Error::new_spanned(
                    ty,
                    format!(
                        "unsupported column type `{other}` — supported: i32, i64, f64, String, bool, Vec<u8>, \
                         DateTime<Utc>, NaiveDate, Uuid, Decimal (the last four behind a `qbrs` feature), \
                         or Option<..> of one of those"
                    ),
                ));
            }
        };
        return Ok(path);
    }
    Err(syn::Error::new_spanned(ty, "unsupported column type"))
}

/// Whether a path segment's single generic argument is the named type.
fn generic_argument_is(seg: &syn::PathSegment, wanted: &str) -> bool {
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return false;
    };
    let mut types = args.args.iter().filter_map(|a| match a {
        syn::GenericArgument::Type(Type::Path(p)) => p.path.segments.last(),
        _ => None,
    });
    match (types.next(), types.next()) {
        (Some(only), None) => only.ident == wanted,
        _ => false,
    }
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
        // A nullable schema column is `Nullable<X>`: the wrapper is what
        // makes a real NULL decode as `None`.
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
            }
            #[doc(hidden)]
            impl ::qbrs::row::NamedSealed for #name {}

            impl ::qbrs::row::Named for #name {
                type Name = #type_name;
                const NAME: &'static str = #col_name_str;
            }
            #[doc(hidden)]
            impl ::qbrs::row::Spelled for #name {}
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

    let col_names: Vec<_> = columns.iter().map(|c| c.field_name.clone()).collect();
    let all_fields = col_names.iter().rev().fold(quote! { Tail }, |tail, name| {
        quote! {
            ::qbrs::row::RowCons<
                columns::#name,
                <::qbrs::expr::Column<columns::#name> as ::qbrs::select::RowField<Scope, Idx>>::Value,
                #tail,
            >
        }
    });

    Ok(quote! {
        #[allow(non_snake_case)]
        pub mod #mod_ident {
            pub struct Table;
            impl ::qbrs::scope::Table for Table {
                const NAME: &'static str = #table_name;
            }
            impl ::qbrs::scope::BaseTableSealed for Table {}
            impl ::qbrs::scope::BaseTable for Table {}

            #[allow(non_camel_case_types)]
            pub mod columns {
                #(#keys)*
            }

            #(#consts)*
            #(#accessors)*

            /// Every column of this table, in declaration order.
            #[allow(non_upper_case_globals)]
            pub const All: ::qbrs::select::All<Table> = ::qbrs::select::All::new();

            // One `Idx` for the whole table: every column of it is found at
            // the same place in the scope, with the same nullability.
            impl<Scope, Idx> ::qbrs::select::AllColumns<Scope, Idx> for Table
            where
                #(::qbrs::expr::Column<columns::#col_names>:
                    ::qbrs::select::RowField<Scope, Idx>,)*
            {
                type Fields<Tail> = #all_fields;
                fn push_items(out: &mut ::std::vec::Vec<::qbrs::render::SelectItem>) {
                    #(out.push(::qbrs::select::RowField::item(&#col_names));)*
                }
            }
        }

        #(#accessor_uses)*
    })
}

/// One column's `row.<name>()` accessor. The `Idx` parameter is the same
/// inferred lookup index `row::Field` and `scope::Find` carry; it can't be
/// hidden, since an impl generic constrained only by a `where` clause isn't
/// accepted. A helper reading two columns needs two of them — one index
/// records one position.
fn accessor_trait(trait_ident: &Ident, method: &Ident, key: &TokenStream2) -> TokenStream2 {
    let method_str = method.to_string();
    let missing = format!("this query's rows have no `{method_str}` field");
    let label = format!(
        "add `{method_str}` to the query's selection list, or read the field that is there"
    );
    quote! {
        #[diagnostic::on_unimplemented(message = #missing, label = #label)]
        pub trait #trait_ident<Idx> {
            type Value;
            fn #method(&self) -> &Self::Value;
        }

        impl<L, Idx> #trait_ident<Idx> for ::qbrs::row::Row<L>
        where
            L: ::qbrs::row::Field<#key, Idx>,
        {
            type Value = <L as ::qbrs::row::Field<#key, Idx>>::Value;
            fn #method(&self) -> &Self::Value {
                self.peek_key::<#key, Idx>()
            }
        }
    }
}

/// `#[derive(FromRow)]`: fills the struct from a `Row` by matching each
/// field's name against the row's keys. The struct itself stays free of
/// column paths and query shape — the only thing it declares is what it
/// wants called what, and `#[from_row(rename = "..")]` where the two names
/// differ.
#[proc_macro_derive(FromRow, attributes(from_row))]
pub fn derive_from_row(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_from_row(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// `#[from_row(rename = "column_name")]` on a field.
fn rename_attr(field: &syn::Field) -> syn::Result<Option<String>> {
    for attr in &field.attrs {
        if !attr.path().is_ident("from_row") {
            continue;
        }
        let mut renamed = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value: syn::LitStr = meta.value()?.parse()?;
                renamed = Some(value.value());
                Ok(())
            } else {
                Err(meta.error("unknown #[from_row(..)] option, expected `rename = \"...\"`"))
            }
        })?;
        return Ok(renamed);
    }
    Ok(None)
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
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "#[derive(FromRow)] doesn't support generic structs: a field's type is what \
             its column must decode to, so it has to be concrete",
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
        let field_name_str = match rename_attr(f)? {
            Some(renamed) => renamed,
            None => field_name.to_string(),
        };
        let type_name = type_level_name(&field_name_str);
        markers.push(quote! {
            pub struct #field_name;
            #[doc(hidden)]
            impl ::qbrs::row::NamedSealed for #field_name {}

            impl ::qbrs::row::Named for #field_name {
                type Name = #type_name;
                const NAME: &'static str = #field_name_str;
            }
            #[doc(hidden)]
            impl ::qbrs::row::Spelled for #field_name {}
        });

        let idx = format_ident!("Idx{position}");
        let marker = quote! { #fields_mod::#field_name };
        bounds.push(quote! {
            #receiver: ::qbrs::row::TakeNamed<#marker, #idx, Value = #field_ty>
        });
        receiver = quote! { <#receiver as ::qbrs::row::TakeNamed<#marker, #idx>>::Rest };

        // Numbered bindings: a bare identifier pattern resolves to a unit
        // struct of that name when one is in scope.
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

/// `with! { struct recent_orders { id: Integer, total: BigInt } }` — declares
/// a CTE's pseudo-table. Generates exactly what `#[derive(Table)]` does — a
/// `Table` marker, per-column `ColumnKey`/`Named` markers, `Column` consts,
/// and accessor traits — plus the `CteShape` impl `cte::with` checks a body
/// against, so a bound CTE is a real table everywhere in the crate.
#[proc_macro]
pub fn with(input: TokenStream) -> TokenStream {
    let decl = parse_macro_input!(input as CteDecl);
    expand_with(decl).into()
}

struct CteDecl {
    name: Ident,
    fields: Vec<(Ident, Type)>,
}

impl syn::parse::Parse for CteDecl {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input.parse::<Token![struct]>()?;
        let name: Ident = input.parse()?;
        let body;
        syn::braced!(body in input);
        let mut fields = Vec::new();
        while !body.is_empty() {
            let field: Ident = body.parse()?;
            body.parse::<Token![:]>()?;
            let ty: Type = body.parse()?;
            fields.push((field, ty));
            if body.is_empty() {
                break;
            }
            body.parse::<Token![,]>()?;
        }
        Ok(CteDecl { name, fields })
    }
}

fn expand_with(decl: CteDecl) -> TokenStream2 {
    let mod_ident = &decl.name;
    let table_name = mod_ident.to_string();

    let mut keys = Vec::new();
    let mut consts = Vec::new();
    let mut accessors = Vec::new();
    let mut accessor_uses = Vec::new();
    let mut names = Vec::new();

    for (field, ty) in &decl.fields {
        let field_str = field.to_string();
        let type_name = type_level_name(&field_str);
        // The marker lives in `columns`, its impls in the enclosing module:
        // a declared column's type is written in the caller's scope, which
        // `use super::*` reaches from here but not from a nested module.
        keys.push(quote! {
            #[derive(Clone, Copy)]
            pub struct #field;
        });
        consts.push(quote! {
            impl ::qbrs::expr::ColumnKey for columns::#field {
                type Table = Table;
                type Sql = #ty;
            }
            #[doc(hidden)]
            impl ::qbrs::row::NamedSealed for columns::#field {}

            impl ::qbrs::row::Named for columns::#field {
                type Name = #type_name;
                const NAME: &'static str = #field_str;
            }
            #[doc(hidden)]
            impl ::qbrs::row::Spelled for columns::#field {}
        });
        consts.push(quote! {
            #[allow(non_upper_case_globals)]
            pub const #field: ::qbrs::expr::Column<columns::#field> =
                ::qbrs::expr::Column::new();
        });
        let trait_ident = format_ident!("Has{}", to_camel_case(&field_str));
        accessors.push(accessor_trait(
            &trait_ident,
            field,
            &quote! { columns::#field },
        ));
        accessor_uses.push(quote! {
            #[allow(unused_imports)]
            pub use #mod_ident::#trait_ident as _;
        });
        names.push(field_str);
    }

    let declared_row =
        decl.fields
            .iter()
            .rev()
            .fold(quote! { ::qbrs::row::RowNil }, |tail, (field, ty)| {
                quote! {
                    ::qbrs::row::RowCons<
                        columns::#field,
                        <#ty as ::qbrs::expr::SqlType>::Native,
                        #tail,
                    >
                }
            });

    let col_idents: Vec<_> = decl.fields.iter().map(|(field, _)| field.clone()).collect();
    let all_fields = col_idents.iter().rev().fold(quote! { Tail }, |tail, name| {
        quote! {
            ::qbrs::row::RowCons<
                columns::#name,
                <::qbrs::expr::Column<columns::#name> as ::qbrs::select::RowField<Scope, Idx>>::Value,
                #tail,
            >
        }
    });

    quote! {
        #[allow(non_snake_case)]
        pub mod #mod_ident {
            use super::*;

            // Deliberately no `BaseTable`: a CTE's pseudo-table is reached
            // through its `cte::with(..)` binding, which is what makes
            // selecting from an unbound one unwritable.
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

            /// Every column of this CTE, in declaration order.
            #[allow(non_upper_case_globals)]
            pub const All: ::qbrs::select::All<Table> = ::qbrs::select::All::new();

            impl<Scope, Idx> ::qbrs::select::AllColumns<Scope, Idx> for Table
            where
                #(::qbrs::expr::Column<columns::#col_idents>:
                    ::qbrs::select::RowField<Scope, Idx>,)*
            {
                type Fields<Tail> = #all_fields;
                fn push_items(out: &mut ::std::vec::Vec<::qbrs::render::SelectItem>) {
                    #(out.push(::qbrs::select::RowField::item(&#col_idents));)*
                }
            }

            impl ::qbrs::cte::CteShape for Table {
                type Row = #declared_row;
                const COLUMN_NAMES: &'static [&'static str] = &[#(#names),*];
            }
        }

        #(#accessor_uses)*
    }
}

/// `label!(rank_in_user, rank_overall);` — declares output-column names for
/// computed selections, in a `label` module so a local binding of the same
/// name can never shadow one. A scope holds one `label` module, so a scope
/// gets one invocation listing every name it needs; an invocation inside the
/// function that runs the query keeps those names next to their use.
#[proc_macro]
pub fn label(input: TokenStream) -> TokenStream {
    let names = parse_macro_input!(input with Punctuated::<Ident, Token![,]>::parse_terminated);
    let mut decls = Vec::new();
    let mut uses = Vec::new();
    for name in &names {
        let name_str = name.to_string();
        let type_name = type_level_name(&name_str);
        let trait_ident = format_ident!("Has{}", to_camel_case(&name_str));
        let accessor = accessor_trait(&trait_ident, name, &quote! { label::#name });
        decls.push(quote! {
            #[allow(non_camel_case_types)]
            #[derive(Clone, Copy)]
            pub struct #name;
            impl ::qbrs::row::RowKey for #name {
                type Key = #name;
            }
            impl ::qbrs::row::LookupKey for #name {}
            impl ::qbrs::expr::LabelKey for #name {}
            #[doc(hidden)]
            impl ::qbrs::row::NamedSealed for #name {}

            impl ::qbrs::row::Named for #name {
                type Name = #type_name;
                const NAME: &'static str = #name_str;
            }
            #[doc(hidden)]
            impl ::qbrs::row::Spelled for #name {}
        });
        uses.push(accessor);
    }
    // The accessor traits sit beside the module rather than inside it with
    // an anonymous re-export, as a schema's do: `label!` is meant to be
    // invoked inside the function that runs the query, and a `use` in a
    // function body cannot name a module declared in that same body.
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
/// not_null, no default   -> `T` (required, so `build()` waits for it)
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
    let builder_ident = format_ident!("{}Builder", insert_ident);
    // One type parameter per required column, `()` until it is given a
    // value and the column's own type after — so `build()` exists exactly
    // when every required column has one, and no value is ever unwrapped.
    let slots: Vec<Ident> = required
        .iter()
        .map(|c| format_ident!("__Qbrs{}", to_camel_case(&c.field_name.to_string())))
        .collect();
    let required_names: Vec<&Ident> = required.iter().map(|c| &c.field_name).collect();
    let required_types: Vec<&syn::Type> = required.iter().map(|c| &c.base_ty).collect();
    let optional: Vec<&&ColumnInfo> = insertable
        .iter()
        .filter(|c| c.nullable || c.has_default)
        .collect();
    let optional_names: Vec<&Ident> = optional.iter().map(|c| &c.field_name).collect();
    let optional_types: Vec<TokenStream2> = optional
        .iter()
        .map(|c| {
            let base = &c.base_ty;
            match (c.nullable, c.has_default) {
                (true, false) => quote! { ::std::option::Option<#base> },
                (false, true) => quote! { ::qbrs::insert::Defaultable<#base> },
                _ => quote! { ::qbrs::insert::Defaultable<::std::option::Option<#base>> },
            }
        })
        .collect();

    // No defaults on these parameters: a defaulted one is elided when the
    // compiler prints the type, and the whole point of the slots is that the
    // printed type says which column is still missing.
    let builder_generics = if slots.is_empty() {
        quote! {}
    } else {
        quote! { <#(#slots),*> }
    };
    let builder_args = if slots.is_empty() {
        quote! {}
    } else {
        quote! { <#(#slots),*> }
    };
    let empty_args = if slots.is_empty() {
        quote! {}
    } else {
        let missing = required_names
            .iter()
            .map(|n| quote! { ::qbrs::insert::Missing<#mod_ident::columns::#n> });
        quote! { <#(#missing),*> }
    };
    let full_args = if slots.is_empty() {
        quote! {}
    } else {
        quote! { <#(#required_types),*> }
    };

    // Setting a required column moves its slot from `()` to its type,
    // leaving the others alone.
    let required_setters = required.iter().enumerate().map(|(i, c)| {
        let name = &c.field_name;
        let base = &c.base_ty;
        let others: Vec<&Ident> = slots
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, s)| s)
            .collect();
        let before: Vec<TokenStream2> = slots
            .iter()
            .enumerate()
            .map(|(j, s)| {
                if j == i {
                    let col = &required_names[j];
                    quote! { ::qbrs::insert::Missing<#mod_ident::columns::#col> }
                } else {
                    quote! { #s }
                }
            })
            .collect();
        let after: Vec<TokenStream2> = slots
            .iter()
            .enumerate()
            .map(|(j, s)| if j == i { quote! { #base } } else { quote! { #s } })
            .collect();
        let carried_required: Vec<TokenStream2> = required_names
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, n)| quote! { #n: self.#n })
            .collect();
        quote! {
            impl<#(#others),*> #builder_ident<#(#before),*> {
                pub fn #name(self, value: impl ::std::convert::Into<#base>) -> #builder_ident<#(#after),*> {
                    #builder_ident {
                        #name: ::std::convert::Into::into(value),
                        #(#carried_required,)*
                        #(#optional_names: self.#optional_names,)*
                    }
                }
            }
        }
    });

    let setters = optional.iter().map(|c| {
        let name = &c.field_name;
        let base = &c.base_ty;
        if c.nullable && !c.has_default {
            quote! {
                pub fn #name(mut self, value: impl ::qbrs::insert::IntoNullable<#base>) -> Self {
                    self.#name = ::qbrs::insert::IntoNullable::into_nullable(value);
                    self
                }
            }
        } else if !c.nullable && c.has_default {
            quote! {
                pub fn #name(mut self, value: impl ::qbrs::insert::IntoDefaultable<#base>) -> Self {
                    self.#name = ::qbrs::insert::IntoDefaultable::into_defaultable(value);
                    self
                }
            }
        } else {
            // Nullable *and* defaulted: three states, so the third one — an
            // explicit NULL, as opposed to letting the schema's default
            // stand — needs a way to be said.
            let null_setter = format_ident!("{}_null", name);
            quote! {
                pub fn #name(
                    mut self,
                    value: impl ::qbrs::insert::IntoDefaultable<::std::option::Option<#base>>,
                ) -> Self {
                    self.#name = ::qbrs::insert::IntoDefaultable::into_defaultable(value);
                    self
                }

                pub fn #null_setter(mut self) -> Self {
                    self.#name = ::qbrs::insert::Defaultable::Value(::std::option::Option::None);
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
            /// Names every column it sets, so two columns of the same type
            /// cannot be handed to each other's position. `build()` appears
            /// once every column without a default has a value.
            pub fn builder() -> #builder_ident #empty_args {
                #builder_ident {
                    #(#required_names: ::qbrs::insert::Missing::new(),)*
                    #(#optional_names: ::std::default::Default::default(),)*
                }
            }
        }

        pub struct #builder_ident #builder_generics {
            #(#required_names: #slots,)*
            #(#optional_names: #optional_types,)*
        }

        #(#required_setters)*

        impl #builder_args #builder_ident #builder_args {
            #(#setters)*
        }

        impl #builder_ident #full_args {
            pub fn build(self) -> #insert_ident {
                #insert_ident {
                    #(#required_names: self.#required_names,)*
                    #(#optional_names: self.#optional_names,)*
                }
            }
        }

        impl ::qbrs::insert::InsertRowSealed for #insert_ident {}

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

        impl ::qbrs::update::UpdateRowSealed for #update_ident {}

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
