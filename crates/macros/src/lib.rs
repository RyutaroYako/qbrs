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
    base_ty: Type,
    sql_marker: TokenStream2,
    nullable: bool,
    primary_key: bool,
    generated: bool,
    has_default: bool,
}

impl ColumnInfo {
    fn insertable(&self) -> bool {
        !self.generated
    }

    fn assignable(&self) -> bool {
        !self.generated && !self.primary_key
    }
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
                    // Not rendered into an `INSERT`: the database applies its own
                    // default, so `default = "expr"` only has to parse.
                    if meta.input.peek(syn::Token![=]) {
                        let _: syn::Expr = meta.value()?.parse()?;
                    }
                } else {
                    return Err(meta.error("unknown #[column(..)] option"));
                }
                Ok(())
            })?;
        }
        // An `Option<T>` primary key is not a nullable column, since SQL forbids NULL
        // in a primary key.
        let nullable = nullable && !primary_key;
        let sql_marker = sql_marker_for(&base_ty)?;
        columns.push(ColumnInfo {
            field_name,
            base_ty,
            sql_marker,
            nullable,
            primary_key,
            generated,
            has_default,
        });
    }

    let schema_mod = gen_schema_mod(&mod_ident, &table_name, &columns)?;
    let insert_struct = gen_insert_struct(struct_ident, &mod_ident, &columns);
    let update_struct = gen_update_struct(struct_ident, &mod_ident, &columns);

    // A derive cannot put `#[allow(dead_code)]` on the struct it is attached to, and a
    // struct nothing constructs warns in every binary crate. Only a struct literal counts
    // as a construction, so this function is what silences it.
    let field_names: Vec<&Ident> = columns.iter().map(|c| &c.field_name).collect();
    let never_constructed = quote! {
        const _: () = {
            #[allow(dead_code)]
            fn __qbrs_schema_is_a_declaration(row: #struct_ident) -> #struct_ident {
                #struct_ident { #(#field_names: row.#field_names,)* }
            }
        };
    };

    Ok(quote! {
        #schema_mod
        #insert_struct
        #update_struct
        #never_constructed
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

// A closed match rather than a trait projection such as `<T as HasSql>::Sql`, so an
// unsupported type is an error naming the field's type, not a failed bound in
// generated code.
fn sql_marker_for(ty: &Type) -> syn::Result<TokenStream2> {
    if let Type::Path(p) = ty
        && let Some(seg) = p.path.segments.last()
    {
        let name = seg.ident.to_string();
        if name == "Vec" {
            return [
                ("u8", quote! { ::qbrs::expr::Bytes }),
                ("String", quote! { ::qbrs::expr::TextArray }),
                ("i32", quote! { ::qbrs::expr::IntegerArray }),
                ("i64", quote! { ::qbrs::expr::BigIntArray }),
                ("Uuid", quote! { ::qbrs::expr::UuidArray }),
            ]
            .into_iter()
            .find_map(|(element, marker)| sole_generic_argument_is(seg, element).then_some(marker))
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    ty,
                    "unsupported column type: a `Vec` is a column type as `Vec<u8>` (bytes) or \
                     as `Vec<String>`/`Vec<i32>`/`Vec<i64>`/`Vec<Uuid>` (a Postgres array). An \
                     array whose elements can be NULL has no column type yet.",
                )
            });
        }
        if name == "DateTime" && !sole_generic_argument_is(seg, "Utc") {
            return Err(syn::Error::new_spanned(
                ty,
                "unsupported column type: `DateTime` is a column type only as `DateTime<Utc>` \
                 (timestamptz)",
            ));
        }
        let path = match name.as_str() {
            "i32" => quote! { ::qbrs::expr::Integer },
            "i64" => quote! { ::qbrs::expr::BigInt },
            "f64" => quote! { ::qbrs::expr::Real },
            "String" => quote! { ::qbrs::expr::Text },
            "bool" => quote! { ::qbrs::expr::Bool },
            // No feature check here: a proc macro cannot see `qbrs`'s features, and
            // without the feature these paths fail to resolve at the marker.
            "DateTime" => quote! { ::qbrs::expr::Timestamptz },
            "NaiveDate" => quote! { ::qbrs::expr::Date },
            "Uuid" => quote! { ::qbrs::expr::Uuid },
            "Decimal" => quote! { ::qbrs::expr::Numeric },
            // Not a bare `Value`: the prelude every schema glob-imports exports qbrs's
            // own bind `Value`, which would then pass for a JSON column.
            "Value" if path_is_qualified_by(p, "serde_json") => quote! { ::qbrs::expr::Json },
            other => {
                return Err(syn::Error::new_spanned(
                    ty,
                    format!(
                        "unsupported column type `{other}`. Supported: i32, i64, f64, String, bool, Vec<u8>, \
                         Vec<String>, Vec<i32>, Vec<i64>, Vec<Uuid>, DateTime<Utc>, NaiveDate, Uuid, Decimal, \
                         serde_json::Value (spelled with its crate; the last five, and Vec<Uuid>, behind a \
                         `qbrs` feature), or Option<..> of one of those"
                    ),
                ));
            }
        };
        return Ok(path);
    }
    Err(syn::Error::new_spanned(ty, "unsupported column type"))
}

fn path_is_qualified_by(p: &syn::TypePath, wanted: &str) -> bool {
    let segments = &p.path.segments;
    segments.len() >= 2 && segments[segments.len() - 2].ident == wanted
}

fn sole_generic_argument_is(seg: &syn::PathSegment, wanted: &str) -> bool {
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
        let base_sql_ty = &c.sql_marker;
        let col_sql_ty = if c.nullable {
            quote! { ::qbrs::scope::Nullable<#base_sql_ty> }
        } else {
            quote! { #base_sql_ty }
        };
        let col_name_str = sql_name(name);
        let type_name = type_level_name(&col_name_str);
        let writable = if c.assignable() {
            quote! {
                #[doc(hidden)]
                impl ::qbrs::expr::WritableSealed for #name {}
                impl ::qbrs::expr::Writable for #name {}
            }
        } else {
            quote! {}
        };
        keys.push(quote! {
            #[derive(Clone, Copy)]
            pub struct #name;
            impl ::qbrs::expr::ColumnKey for #name {
                type Table = super::Table;
                type Sql = #col_sql_ty;
            }
            #writable
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
    let cons_list = |names: &[proc_macro2::Ident]| {
        names.iter().rev().fold(
            quote! { ::qbrs::scope::Nil },
            |tail, name| quote! { ::qbrs::scope::Cons<::qbrs::expr::Column<columns::#name>, #tail> },
        )
    };
    let column_list = cons_list(&col_names);
    let written_names: Vec<_> = columns
        .iter()
        .filter(|c| c.insertable())
        .map(|c| c.field_name.clone())
        .collect();
    let written_list = cons_list(&written_names);
    // Projected through the column's `Sql` rather than copying the field's tokens, which
    // resolve only in the caller's scope: a `use chrono::DateTime` there is not in scope
    // inside the generated module.
    let all_row = columns
        .iter()
        .rev()
        .fold(quote! { ::qbrs::row::RowNil }, |tail, c| {
            let name = &c.field_name;
            quote! {
                ::qbrs::row::RowCons<
                    columns::#name,
                    <<columns::#name as ::qbrs::expr::ColumnKey>::Sql
                        as ::qbrs::expr::SqlType>::Native,
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

            /// What `select(All)` decodes to with this table joined
            /// not-null: the type a stored `Prepared`/`DynSelect` names,
            /// rather than a hand-written `RowCons` chain.
            pub type AllRow = ::qbrs::row::Row<#all_row>;

            #[doc(hidden)]
            impl ::qbrs::select::SelectableSealed for Table {}

            impl ::qbrs::select::AllColumns for Table {
                type Columns = #column_list;
            }

            impl ::qbrs::insert::WrittenColumns for Table {
                type Columns = #written_list;
            }
        }

        #(#accessor_uses)*
    })
}

// `Idx` is a trait parameter rather than hidden in the impl, since an impl generic
// constrained only by a `where` clause is rejected.
// The bound stays on the impl rather than moving to the method as the execution
// terminals' does: an unconditional impl puts every schema's `.total()` on every `Row`,
// and two tables with a same-named column make each call ambiguous.
fn accessor_trait(trait_ident: &Ident, method: &Ident, key: &TokenStream2) -> TokenStream2 {
    quote! {
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
/// column paths and query shape. The only thing it declares is what it
/// wants called what, and `#[from_row(rename = "..")]` where the two names
/// differ.
#[proc_macro_derive(FromRow, attributes(from_row))]
pub fn derive_from_row(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_from_row(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

enum FieldSource {
    Named(String),
    // Not redundant with `Named`: when two selected columns share a name, only the
    // column's identity tells them apart.
    Column(syn::Path),
}

fn field_source(field: &syn::Field, field_name: &Ident) -> syn::Result<FieldSource> {
    let mut renamed = None;
    let mut from = None;
    for attr in &field.attrs {
        if !attr.path().is_ident("from_row") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value: syn::LitStr = meta.value()?.parse()?;
                renamed = Some(value.value());
                Ok(())
            } else if meta.path.is_ident("from") {
                from = Some(meta.value()?.parse::<syn::Path>()?);
                Ok(())
            } else {
                Err(meta.error(
                    "unknown #[from_row(..)] option, expected `rename = \"...\"` or `from = <table>::<column>`",
                ))
            }
        })?;
    }
    match (renamed, from) {
        (Some(_), Some(path)) => Err(syn::Error::new_spanned(
            path,
            "a field is filled by name or by column, not both: drop the `rename`",
        )),
        (_, Some(path)) => Ok(FieldSource::Column(path)),
        (Some(renamed), None) => Ok(FieldSource::Named(renamed)),
        (None, None) => Ok(FieldSource::Named(sql_name(field_name))),
    }
}

fn column_key_path(path: &syn::Path) -> syn::Result<syn::Path> {
    let mut key = path.clone();
    let last = key
        .segments
        .pop()
        .ok_or_else(|| syn::Error::new_spanned(path, "expected `<table>::<column>`"))?;
    if key.segments.is_empty() {
        return Err(syn::Error::new_spanned(
            path,
            "expected `<table>::<column>`, so the column's table is named too",
        ));
    }
    key.segments
        .push(syn::PathSegment::from(format_ident!("columns")));
    key.segments.push(last);
    Ok(key)
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

    // Markers rather than bounds on the spelled name directly, so a failed lookup
    // reports `user_summary_fields::email` instead of a `NameChar` chain.
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
        let idx = format_ident!("Idx{position}");
        // Not the field's own name: a bare identifier pattern matches a unit struct or a
        // `const` of that name when one is in scope, such as a schema's column.
        let binding = format_ident!("__field{position}");

        match field_source(f, &field_name)? {
            FieldSource::Named(field_name_str) => {
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

                    impl ::qbrs::row::FieldValue for #field_name {
                        type Value = #field_ty;
                    }
                });

                let marker = quote! { #fields_mod::#field_name };
                bounds.push(quote! {
                    #receiver: ::qbrs::row::TakeNamed<#marker, #idx, Value = #field_ty>
                });
                receiver = quote! { <#receiver as ::qbrs::row::TakeNamed<#marker, #idx>>::Rest };
                steps.push(quote! {
                    let (#binding, row) = row.take_named::<#marker, #idx>();
                });
            }
            FieldSource::Column(path) => {
                let key = column_key_path(&path)?;
                bounds.push(quote! {
                    #receiver: ::qbrs::row::Field<#key, #idx, Value = #field_ty>
                });
                receiver = quote! { <#receiver as ::qbrs::row::Field<#key, #idx>>::Rest };
                steps.push(quote! {
                    let (#binding, row) = row.take_key::<#key, #idx>();
                });
            }
        }
        inits.push(quote! { #field_name: #binding });
        idx_params.push(idx);
    }

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        mod #fields_mod {
            // Not unused: a field's declared type is written in the caller's scope, which
            // this module reaches only through `super`.
            #[allow(unused_imports)]
            use super::*;

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

/// `with! { struct recent_orders { id: Integer, total: BigInt } }` declares
/// a CTE's pseudo-table. It generates exactly what `#[derive(Table)]` does
/// (a `Table` marker, per-column `ColumnKey`/`Named` markers, `Column`
/// consts, and accessor traits), plus the `CteShape` impl `cte::with` checks
/// a body against, so a bound CTE is a real table everywhere in the crate.
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
    let mut key_impls = Vec::new();
    let mut consts = Vec::new();
    let mut accessors = Vec::new();
    let mut accessor_uses = Vec::new();
    let mut names = Vec::new();

    for (field, ty) in &decl.fields {
        let field_str = sql_name(field);
        let type_name = type_level_name(&field_str);
        keys.push(quote! {
            #[derive(Clone, Copy)]
            pub struct #field;
        });
        key_impls.push(quote! {
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
    let column_list = col_idents.iter().rev().fold(
        quote! { ::qbrs::scope::Nil },
        |tail, name| quote! { ::qbrs::scope::Cons<::qbrs::expr::Column<columns::#name>, #tail> },
    );

    quote! {
        #[allow(non_snake_case)]
        pub mod #mod_ident {
            use super::*;

            // No `BaseTable` impl, unlike a schema's: the pseudo-table is reached through
            // its `cte::with(..)` binding, which is what makes selecting from an unbound
            // CTE unwritable.
            pub struct Table;
            impl ::qbrs::scope::Table for Table {
                const NAME: &'static str = #table_name;
            }

            #[allow(non_camel_case_types)]
            pub mod columns {
                #(#keys)*
            }

            // The impls stay out of `columns`, unlike `#[derive(Table)]`'s: a declared
            // type is written in the caller's scope, which `use super::*` reaches from
            // here but not from a nested module.
            #(#key_impls)*
            #(#consts)*
            #(#accessors)*

            /// Every column of this CTE, in declaration order.
            #[allow(non_upper_case_globals)]
            pub const All: ::qbrs::select::All<Table> = ::qbrs::select::All::new();

            /// What `select(All)` decodes to. A CTE's pseudo-table names
            /// its row the way a real one does.
            pub type AllRow = ::qbrs::row::Row<#declared_row>;

            #[doc(hidden)]
            impl ::qbrs::select::SelectableSealed for Table {}

            impl ::qbrs::select::AllColumns for Table {
                type Columns = #column_list;
            }

            impl ::qbrs::cte::CteShape for Table {
                type Row = #declared_row;
            }
        }

        #(#accessor_uses)*
    }
}

/// `label!(rank_in_user, rank_overall);` declares output-column names for
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
        let name_str = sql_name(name);
        let type_name = type_level_name(&name_str);
        let trait_ident = format_ident!("Has{}", to_camel_case(&name_str));
        let accessor = accessor_trait(&trait_ident, name, &quote! { label::#name });
        decls.push(quote! {
            #[allow(non_camel_case_types)]
            #[derive(Clone, Copy)]
            pub struct #name;
            #[diagnostic::do_not_recommend]
            impl ::qbrs::row::RowKey for #name {
                type Key = #name;
            }
            #[diagnostic::do_not_recommend]
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
    quote! {
        pub mod label {
            #(#decls)*
        }
        #(#uses)*
    }
    .into()
}

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

fn gen_insert_struct(
    struct_ident: &Ident,
    mod_ident: &Ident,
    columns: &[ColumnInfo],
) -> TokenStream2 {
    let insert_ident = format_ident!("{}Insert", struct_ident);
    let insertable: Vec<&ColumnInfo> = columns.iter().filter(|c| c.insertable()).collect();

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
    let slots: Vec<Ident> = required
        .iter()
        .map(|c| format_ident!("__Qbrs{}", to_camel_case(&sql_name(&c.field_name))))
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

    // No defaults on these parameters: the compiler elides a defaulted one when it prints
    // the type, and the printed type is what says which column is still missing.
    let slot_params = if slots.is_empty() {
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
                pub fn #name(self, value: impl ::qbrs::insert::IntoColumnValue<#base>) -> #builder_ident<#(#after),*> {
                    #builder_ident {
                        #name: ::qbrs::insert::IntoColumnValue::into_column_value(value),
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
                pub fn #name(
                    mut self,
                    value: impl ::qbrs::insert::IntoColumnValue<::std::option::Option<#base>>,
                ) -> Self {
                    self.#name = ::qbrs::insert::IntoColumnValue::into_column_value(value);
                    self
                }
            }
        } else if !c.nullable && c.has_default {
            quote! {
                pub fn #name(
                    mut self,
                    value: impl ::qbrs::insert::IntoColumnValue<::qbrs::insert::Defaultable<#base>>,
                ) -> Self {
                    self.#name = ::qbrs::insert::IntoColumnValue::into_column_value(value);
                    self
                }
            }
        } else {
            // Not covered by `name(None)`, which leaves the default standing as `None` does at
            // every setter, so an explicit NULL needs a setter of its own.
            let null_setter = format_ident!("{}_null", name);
            quote! {
                pub fn #name(
                    mut self,
                    value: impl ::qbrs::insert::IntoColumnValue<
                        ::qbrs::insert::Defaultable<::std::option::Option<#base>>,
                    >,
                ) -> Self {
                    self.#name = ::qbrs::insert::IntoColumnValue::into_column_value(value);
                    self
                }

                pub fn #null_setter(mut self) -> Self {
                    self.#name = ::qbrs::insert::Defaultable::Value(::std::option::Option::None);
                    self
                }
            }
        }
    });

    // No `Insertable` for a table whose every column is generated: its one row is
    // `DEFAULT VALUES`, which the bulk paths cannot repeat.
    let insertable_marker = if insertable.is_empty() {
        quote! {}
    } else {
        quote! {
            #[doc(hidden)]
            impl ::qbrs::insert::InsertableSealed for #insert_ident {}
            impl ::qbrs::insert::Insertable for #insert_ident {}
        }
    };
    // One chain rather than a header list beside a value list, which could disagree in
    // length.
    let insert_values_ty =
        insertable
            .iter()
            .rev()
            .fold(quote! { ::qbrs::row::RowNil }, |tail, c| {
                let name = &c.field_name;
                quote! {
                    ::qbrs::row::RowCons<
                        #mod_ident::columns::#name,
                        ::qbrs::insert::InsertValue,
                        #tail,
                    >
                }
            });
    let cell_values: Vec<TokenStream2> = insertable.iter().map(|c| {
        let name = &c.field_name;
        let sql_ty = &c.sql_marker;
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
    }).collect();

    let insert_values_expr = cell_values.iter().rev().fold(
        quote! { ::qbrs::row::RowNil },
        |tail, value| quote! { ::qbrs::row::RowCons::new(#value, #tail) },
    );

    quote! {
        #[derive(::std::fmt::Debug, ::std::clone::Clone)]
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

        pub struct #builder_ident #slot_params {
            #(#required_names: #slots,)*
            #(#optional_names: #optional_types,)*
        }

        #(#required_setters)*

        impl #slot_params #builder_ident #slot_params {
            #(#setters)*
        }

        #(
            impl ::qbrs::insert::Filled<#mod_ident::columns::#required_names> for #required_types {
                type Value = #required_types;
                fn filled(self) -> #required_types {
                    self
                }
            }
        )*

        // The bound is on the method rather than the impl: on the impl, an incomplete row
        // makes `build` disappear instead of naming the missing column.
        impl #slot_params #builder_ident #slot_params {
            /// Callable once every column without a default has a value. A missing one is
            /// named in the error.
            pub fn build(self) -> #insert_ident
            where
                #(#slots: ::qbrs::insert::Filled<
                    #mod_ident::columns::#required_names,
                    Value = #required_types,
                >,)*
            {
                #insert_ident {
                    #(#required_names: ::qbrs::insert::Filled::filled(self.#required_names),)*
                    #(#optional_names: self.#optional_names,)*
                }
            }
        }

        impl ::qbrs::insert::InsertRowSealed for #insert_ident {}

        #insertable_marker

        impl ::qbrs::insert::InsertRow for #insert_ident {
            type Table = #mod_ident::Table;
            type Values = #insert_values_ty;

            fn into_values(self) -> Self::Values {
                #insert_values_expr
            }
        }
    }
}

fn gen_update_struct(
    struct_ident: &Ident,
    mod_ident: &Ident,
    columns: &[ColumnInfo],
) -> TokenStream2 {
    let update_ident = format_ident!("{}Update", struct_ident);
    let updatable: Vec<&ColumnInfo> = columns.iter().filter(|c| c.assignable()).collect();

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
        let col_name_str = sql_name(&c.field_name);
        let sql_ty = &c.sql_marker;
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

    let builder_ident = format_ident!("{}Builder", update_ident);
    let setters = updatable.iter().map(|c| {
        let name = &c.field_name;
        let base = &c.base_ty;
        if c.nullable {
            // Takes `Option<T>` rather than the field's `Option<Option<T>>`: `None` means
            // nothing to set, as at every other setter, and an explicit NULL is `_null`.
            let null_setter = format_ident!("{}_null", name);
            quote! {
                pub fn #name(
                    mut self,
                    value: impl ::qbrs::insert::IntoColumnValue<::std::option::Option<#base>>,
                ) -> Self {
                    self.0.#name = ::qbrs::insert::IntoColumnValue::into_column_value(value)
                        .map(::std::option::Option::Some);
                    self
                }

                pub fn #null_setter(mut self) -> Self {
                    self.0.#name = ::std::option::Option::Some(::std::option::Option::None);
                    self
                }
            }
        } else {
            quote! {
                pub fn #name(
                    mut self,
                    value: impl ::qbrs::insert::IntoColumnValue<::std::option::Option<#base>>,
                ) -> Self {
                    self.0.#name = ::qbrs::insert::IntoColumnValue::into_column_value(value);
                    self
                }
            }
        }
    });

    quote! {
        #[derive(::std::default::Default, ::std::fmt::Debug, ::std::clone::Clone)]
        pub struct #update_ident {
            #(#fields,)*
        }

        impl #update_ident {
            /// Every setter takes what the column holds or an `Option` of
            /// it, so a request struct's fields map across one for one.
            /// The struct literal's `Option<Option<T>>` is a nullable
            /// column's three states written out, and is easy to nest
            /// wrongly.
            pub fn builder() -> #builder_ident {
                #builder_ident(::std::default::Default::default())
            }
        }

        #[derive(::std::fmt::Debug, ::std::clone::Clone)]
        pub struct #builder_ident(#update_ident);

        impl #builder_ident {
            #(#setters)*

            pub fn build(self) -> #update_ident {
                self.0
            }
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

fn sql_name(ident: &Ident) -> String {
    let spelled = ident.to_string();
    spelled
        .strip_prefix("r#")
        .map(str::to_string)
        .unwrap_or(spelled)
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
