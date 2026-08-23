# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

qbrs is a Drizzle-flavored, compile-time-checked SQL query builder for Rust (not an ORM,
not a raw-SQL macro). The core claim it exists to defend: column/join validity is checked
at compile time **without** giving up dynamic query composition, and compile times stay
linear as join count grows (unlike diesel's documented blowup at ~7 joins). Read `README.md`
first — it states the design goals and the current dialect/feature status matrix.

## Commands

```sh
cargo test --workspace --all-features   # everything, including real-Postgres tests
cargo clippy --workspace --all-targets --all-features -- -D warnings   # CI gate
cargo fmt --all -- --check                              # CI gate
```

Narrower loops:

```sh
cargo test -p qbrs-core                                     # pure SQL-rendering tests, no DB
cargo test -p qbrs-core --test select_smoke                 # one test binary
cargo test -p qbrs-core --test select_smoke left_join_renders_and_typechecks   # one test
cargo test -p qbrs-sqlx                                     # real-Postgres integration tests
cargo run -p qbrs-examples --example 02_select_join          # one runnable example
cargo build -p compile-bench --bin joins_40                  # scope-resolution depth
cargo build -p compile-bench --bin join_chain_20             # ...through a real builder chain
cargo build -p compile-bench --bin cols_16                   # ...and selection width
cargo test -p dialect-exec                                   # rendered SQL, run by a real SQLite
```

**No database setup is needed anywhere.** Tests and examples that need Postgres start their
own throwaway PostgreSQL 17.5, linked into the binary via `pglite-rs` (multi-process mode, a
real postmaster over a unix socket). Nothing is downloaded at test time and Docker is not
involved. Set `DATABASE_URL` to run the same tests/examples against an external Postgres
instead — integration tests are written to be idempotent (`DROP TABLE IF EXISTS` first) so
they work against a persistent server too.

The embedded-server guard value must stay bound for the whole test/example body
(`let (pool, _db) = setup_db().await;`) — dropping it kills the postmaster out from under
the pool. `qbrs-sqlx` tests must call `common::shutdown(pool, guard)` at the end, otherwise
the child process outlives the test binary.

## Workspace layout

- `crates/core` (`qbrs-core`) — all the type-level machinery and SQL rendering. No I/O, no
  async, no driver dependency, and no dependency at all unless a schema asks for one (the
  `chrono`/`uuid`/`decimal` features).
- `crates/macros` (`qbrs-macros`) — `#[derive(Table)]`, `#[derive(FromRow)]`, `label!`,
  and `with!`. All four synthesize identifiers (`HasEmail` from `email`) or a name's
  type-level spelling, which is why none can be `macro_rules!`; `sql!` and `prepare!`, which
  need neither, stay in `core` (`raw.rs`, `prepare.rs`).
- `crates/qbrs` — facade; two `pub use` lines. Users depend on this.
- `crates/qbrs-sqlx` — Postgres execution via `sqlx`. Owns *only* value binding and row
  decoding; no query-building logic belongs here.
- `examples` (`qbrs-examples`) — one numbered runnable example per feature, sharing a schema
  and `setup_db()` from `examples/src/lib.rs`.
- `tests/compile-bench` — synthetic 100-table schema and escalating join-count binaries; this
  is what backs the "linear at 40+ joins" claim in the README.
- `tests/dialect-exec` — every rendered statement shape run against an in-memory SQLite, so
  the `Sqlite` dialect's SQL is checked by SQLite rather than by a string assertion. Add a
  shape here whenever one is added to the renderer.

## Architecture

### The scope mechanism (`core/src/scope.rs`)

This is the load-bearing piece; understand it before touching anything type-level.

A query's FROM/JOIN set is a **flat type-level cons-list**: `Cons<TableSlot<T, N>, Tail>` /
`Nil`, where `N` is `NotNull` or `MaybeNull`. Lookups go through frunk-style *indexed*
traits:

- `Find<T, Index>` — proof `T` is in scope, plus its join-derived `Nullability`.
- `Superset<Req, Idxs>` — proof a whole `Req` list is in scope; used wherever a
  pre-built `Expr` enters a query. Carries the same `Proof` seal `Find` does, and needs it
  more: `Idxs` is a free slot, so a local type there is all the orphan rule asks for, and
  this is the trait the builder bounds actually name.
- `MapNullable` — RIGHT/FULL JOIN flipping every already-joined table to `MaybeNull`.
- `WrapNullable<N>` — idempotently wraps a SQL type as `Nullable<T>`.

The `Index`/`Idxs` parameters are never written by callers (always inferred) but **must** be
trait parameters, not `where`-clause-only generics — an index constrained only by a `where`
clause doesn't compile. The `Here`/`There<I>` encoding is what disambiguates `Find`'s two
impls structurally, rather than relying on shape-based negative reasoning.

Coherence constraint worth memorizing: there must be **no** blanket
`impl<T> WrapNullable<MaybeNull> for T` — it overlaps the `Nullable<T>` impl. Every concrete
base SQL type gets its own non-generic impl, generated by the `sql_leaf_type!` macro in
`expr.rs`. Adding a new SQL leaf type means going through that macro.

Compile-time cost is the reason the list is flat rather than a join tree. Any change that
makes scope resolution non-linear defeats the project's main differentiator — re-check with
`tests/compile-bench` (`joins_05` … `joins_100_full_superset`).

### Static vs. dynamic split

The deliberate rule: **what's joined/selected is a compile-time question; how many predicates
you've added is a runtime one.** `Select`'s `wheres` is a plain `Vec<ExprKind>`, so
`.filter()` returns `Self` and can be called conditionally or in a loop with no escape hatch.
Only conditional *joining* needs `.erase()` into `DynSelect`, which erases the join skeleton
alone and offers no further `.filter()`/`.join()`.

### Rows are keyed by column (`core/src/row.rs`)

A tuple selection's `Selection::Output` is `Row<RowCons<Key, Value, ..>>`, not a tuple.
Each element contributes a key via `select::RowField`: a `Column<C>` keys on its
`expr::ColumnKey` marker `C`, `count()`/the window functions key on the function itself
(`expr::Count`, `window::RowNumber`, ...), a `label!`-declared `expr::LabelKey` overrides
whichever was there, and a bare `Expr` gets `row::Anon`, which is deliberately not
`row::Spelled` — so it can't be named at a `.get()` call, and two anonymous columns don't
stand in for each other in a `UNION` or a CTE body. `row::Field<K, Idx>` is `scope::Find` in a
different costume: same `Here`/`There<I>` indexed lookup, same reason the index must be a
trait parameter.

A single un-tupled selection (`select(users::email)`) stays a bare value — there is no
position to disambiguate, so there is nothing to key. `Row::into_tuple` and
`Vec<Row<_>>::into_tuples` give the positional view back; the
arity limit lives in `row::Prepend`'s impls and nowhere else.

Two lookups, deliberately distinct. `Field` searches by key *identity*
(`row.get(users::email)` must not also match `archived_users::email`); `TakeNamed` searches
by `row::Named`, an identifier spelled one `char` per cell (`NameChar<'e', ..>`), which is
what lets `#[derive(FromRow)]` fill a struct that has never been told a column path. `char`
is one of the three types stable const generics accept — a `&'static str` parameter is not
— and that spelling is why `label!`, `#[derive(Table)]`, and `#[derive(FromRow)]` all have
to be proc macros. Keep the spelling out of diagnostics: it lives in `Named::Name`, and
`TakeNamed` reports the `FromRow` field marker instead.

A field says which column fills it by name, by `#[from_row(rename = "..")]`, or by
`#[from_row(from = users::id)]`, which routes that one field through `Field` (identity)
instead of `TakeNamed` (name) — the only lookup that stays unambiguous when a selection
holds two columns of the same name, and what makes `select((a::All, b::All))` fillable.
The attribute takes the column path a call site writes; `<table>::<column>` and
`<table>::columns::<column>` are the value and the type of one thing, which `#[derive(Table)]`
and `with!` both arrange. A `label!` name is one item that is both, so it has no identity
route and is matched by name.

`FromRow` is its own trait rather than `From`: the per-field lookup indices have nowhere to
live in a foreign trait's fixed shape. Hence `into_struct`/`into_structs`.

Two selections are compared by `row::SameShape` — one cell-by-cell walk checking name and
value together, so a selection wider than the positional view's 16 fields still compares.
Values alone would let a `UNION` branch or a CTE body whose columns merely happen to be
type-compatible splice in transposed, and the result is then read by key. `SameNameAs`
carries `#[diagnostic::do_not_recommend]` so the reported obligation is the two columns,
not the `NameChar` spelling behind them.

One consequence to preserve: `render::SelectItem` carries an optional `AS` label, so a selection list
is `&[SelectItem]` rather than `&[ExprKind]` — `render::render_select_list` is the single
place that renders one, shared by `SELECT` and all three `RETURNING` builders.

### Expressions and rendering stay non-generic

`Expr<Req, S>` is a phantom-tagged wrapper around `ExprKind`, a plain closed enum with no
generics, and `Value` is a closed enum (not `Box<dyn ToSql>`). This keeps `render_expr` a
single function monomorphized once per *dialect*, never per query shape. Preserve this: don't
add generics to `ExprKind`/`Value`, and don't make the renderer generic over query types.

`ExprKind` is `pub(crate)` and `Expr::from_kind` is too, so the typed wrapper is the only way
to build one — that is what makes the `Req`/`S` tags mean anything rather than merely exist.
`expr::raw_expr` and `expr::placeholder` are the two `#[doc(hidden)]` doors, needed because
`sql!` and `prepare!` expand in the caller's crate. A `sql!` slot takes an `expr::RawArg` — a value, or an expression the renderer
writes out — so a column in a slot is quoted by the same code that quotes it anywhere else and
carries its table into the fragment's `Req` (`expr::RawArgs` unions the slots' `Req`s). Only
the text *between* slots is unchecked. A fragment is selectable as written, since `sql!`'s
first argument *is* the decoded type; what still needs `decodes_as::<S>()` is an expression
whose `S` the builder inferred, which can contradict the join.

### Dialects and capability gating

`Dialect` is sealed (`Postgres`/`MySql`/`Sqlite`). Dialect-specific SQL is gated by separate
marker traits — `SupportsReturning`, `SupportsOnConflict`, `SupportsRightJoin`,
`SupportsFullOuterJoin` — so an unsupported call is a compile error. Keep capabilities
fine-grained: they were split precisely because MySQL has `RIGHT JOIN` but not `FULL JOIN`.

### Embedded SQL goes through `Fragment`

A CTE body and a `UNION` branch are the same thing: SQL rendered before its final
placeholder numbering is known, because that depends on how much of the host query has been
rendered. That's `render::Fragment` — text and bind values together, spliced with
`Fragment::splice_into`, which assigns the numbering. `Select::fragment` is the only way to
make one from a query, so no call site has to remember that an embedded query's placeholders
are written by position and numbered later. Both carriers — `Cte<D, _>` and `SetOp<D, _>` —
keep the dialect they were rendered in, which is what makes a fragment safe to hold.

An `EXISTS` subquery is deliberately *not* one: `ExprKind::Exists` holds the `SelectBody`
and its selection unrendered, so its placeholders are numbered by the statement it lands in.
Rendering late is not enough on its own, though — the subquery was capability-checked against
its own dialect and any CTE it binds is already a `Fragment` in that dialect — so
`Select::exists` returns a `select::Exists<D, Req>`, which is a `Condition<D, ..>` and
nothing else. That is why `Condition` and `Predicate` both carry `D`: every boolean clause
goes through them, and they are the only places a dialect-pinned condition could otherwise
be laundered into a dialect-free one. `sql!{}` is likewise an `ExprKind::Template`, its slots rendered
with everything else. If you add another place that embeds a *query* in an expression, hold
the query; if you add one that embeds SQL in a dialect-tagged builder, take a `Fragment` —
don't reintroduce a bare `(String, Vec<Value>)` pair.

Every clause that takes a condition goes through `select::Condition` and comes out a
`Predicate<D, Scope>` — `.filter`, `.having`, and all four joins' `ON`, where the scope
discharged against is the one the join produces. A boolean is `expr::BoolLike`, so a
`Nullable<Bool>` column is a condition on its own. A `Predicate` carries `D` because
discharging gives up the tables a condition named, never the dialect it was built for —
which is what keeps `select::Exists` pinned once it becomes one.

### Builder shape

Builders read in SQL keyword order and take tables as **values**, not turbofish:
`select((users::email,)).from(users::Table).left_join(orders::Table, ..)`. The dialect is a
value too, at whichever terminal decides it: `.to_sql(Postgres)`/`.count_sql(Postgres)`/
`.prepare::<Params, _>(Postgres)` take one, and `.load(&pool)` infers it from the executor,
so `D` is never a turbofish. `Dialect: Default` exists for the generic call sites inside the
crate, which have the type but no value.
`SelectSeed` holds just the selection until `.from()` supplies the initial `Scope`; the
selection is validated against the *final* scope once, at the terminal method
(`.to_sql()` / `.load()`). Every builder carries `PhantomData<fn() -> (D, Scope, ...)>`;
`clippy::type_complexity` is allowed crate-wide in `core/src/lib.rs` for exactly this reason.

Every clause of a `SELECT` other than the selection list lives in one `SelectBody`, which
`Select` and `DynSelect` both hold whole and `SelectBody::render` renders. Add a new clause
there and it flows through `.erase()`, `retype()`, and rendering on its own — don't spread
clause fields back across the two builders or pass them as separate render arguments.

`select::Selection`/`SelectionPart`/`RowField`/`AllColumns`/`ColumnList`, `expr::RawArg` and
`cte::CteShape` are sealed
for the reason `InsertRow` is: each pairs a type-level claim with the runtime list that is
supposed to match it, and a hand-written impl could select a row that decodes transposed.
`select::SelectableSealed` is the `#[doc(hidden)] pub` half, since the derive emits
`AllColumns`/`CteShape` in the schema's own crate. `ColumnList` and `RawArg` need no such
door — nothing outside this crate implements them — and both need the seal: each pairs a
type-level claim with runtime data (`ColumnList` the row against the pushed items, `RawArg`
a slot's `Req` against the column it delegates to), which is precisely what splitting
`AllColumns` was meant to remove.

### Derive and codegen (`crates/macros`)

A generated module can't see the caller's imports, so either the module takes them
(`use super::*;`, which `with!` and `#[derive(FromRow)]`'s field-marker module do) or
nothing the caller wrote is re-resolved inside it: `AllRow`'s field types are projected through `<C as ColumnKey>::Sql`'s `Native`
rather than copied from the field's tokens, which is what lets a column be declared
`DateTime<Utc>` rather than `chrono::DateTime<chrono::Utc>`. A field's SQL name comes from
`sql_name`, which takes off the `r#` a Rust keyword needs — `r#type` is a column called
`type`.

`#[derive(Table)]` generates a `mod users { struct Table; mod columns { struct id; }
const id: Column<columns::id>; trait HasId<Idx> { fn id(&self) -> &Self::Value } }` plus
`UsersInsert`/`UsersUpdate` companions. The `columns` marker is the column's *identity* —
`Column<C>` carries nothing else, so a column's table and SQL type have one source of
truth rather than three positions that can disagree. Accessor traits are re-exported next
to the struct as `pub use users::HasId as _;`: anonymous, so a schema adds exactly zero
names to its module and `use crate::schema::*;` is all a call site needs.

It also emits `const All` (a `select::All<Table>`) and the `select::AllColumns` impl behind
it — which states only `type Columns`, the table's columns as a type-level list, since
`select::ColumnList` walks that one list for both the row's fields and the rendered items;
stating the two separately is what let a hand-written impl select a row that decodes
transposed — so `select(users::All)` never restates the column list, plus `type AllRow` — what that
selection decodes to with the table joined not-null, so a stored `Prepared`/`DynSelect`
names a row instead of spelling a `RowCons` chain by hand. A selection list is a chain of
`select::SelectionPart`s, each contributing `Fields<Tail>` in front of whatever the rest of
the list contributes — which is what lets one tuple element carry a whole table, and makes
the 16-element limit count tables rather than columns.

`label!(rank_in_user, ..)` generates the same shape for a computed column, in a fixed
`label` module so a same-named local binding can never shadow it. One invocation per scope
(a second one collides on `mod label`); declaring it inside the function that runs the
query is the intended usage and sidesteps that. Nullability comes from `Option<T>` wrapping (no
separate attribute); attributes are only `#[column(primary_key | generated | default)]`.
Every column setter takes `insert::IntoColumnValue<Field>` — one trait for all four field
shapes, so two same-typed columns accept the same values however they are declared.
A table whose every column is generated leaves an `INSERT` with no column to name, which SQL
spells `DEFAULT VALUES` (`() VALUES ()` in MySQL) and spells for exactly one row — so the
bulk paths take `insert::Insertable`, which the derive emits only when there is a column to
repeat.

`*Insert` is built through a type-state builder: one generic slot per column that is neither
nullable nor defaulted, `insert::Missing<C>` until that column is given a value and its own
type after. `build()`'s bound — `insert::Filled<C>` per slot, on the method rather than on
the impl — is what makes an incomplete row report *which* column is missing instead of
making `build` disappear. Insert fields use `Defaultable<T>`
(and `Defaultable<Option<T>>` for nullable-with-default) so
omit / explicit-NULL / explicit-value stay distinguishable; update fields use `Option<T>` /
`Option<Option<T>>` for untouched / set-NULL / set-value — reached through
`*Update::builder()`, whose setters take the same `IntoColumnValue` shape the insert
builder's do, so a request's `Option<T>` maps across without the nesting (and the nesting is
what rustc's own "try wrapping in `Some`" turns into a silent `SET column = NULL`). A statement with nothing in it —
`UPDATE .. SET` with no assignments, `INSERT` with no rows — has no SQL form, and both
shapes are ordinary request-shaped data rather than bugs, so the two places that read such
data — `Assignments::from_row(..)` and `.values_all(..)` — return a `Result`
(`NothingToSet` / `NothingToInsert`). Everything downstream takes the non-empty
`update::Assignments` they produce and is infallible: no builder method returns a `Result`
that cannot be `Err`. A column assigned twice keeps the last assignment, since a `SET` list
naming one column twice is SQL no database accepts.

A set-operation chain is a left fold, and SQL's precedence isn't: `INTERSECT` binds tighter
than `UNION`/`EXCEPT`, and SQLite reads all of them left to right. `SetOp::render_branches`
parenthesises the accumulator wherever the operator changes, so the rendered statement says
the fold outright and means the same thing in every dialect — no precedence table anywhere.

`with!{}` generates the same shape for a CTE pseudo-table — markers, consts, accessor traits
and `AllRow` alike — so a CTE *is* a real table to `Scope`/`Find`/`Superset` with no separate
virtual-table machinery. It deliberately does *not* emit `scope::BaseTable`, so the
pseudo-table is not itself a `JoinSource`: the `cte::with(..)` binding is, and it is passed
to `.from(binding)`/`.inner_join(binding, on)` exactly where a table would go, making
attaching the `WITH` clause and putting the pseudo-table in scope one act. Selecting
from a CTE nobody bound, binding one and selecting from another, and splicing a body
rendered for one dialect into another's statement are all unwritable as a result. Its `WITH name (..)` header is read off that same declared row by `row::ColumnNames`, so the
names the outer query reads by and the shape the body was checked against are one fact. The
declared columns are checked against the actual body at `cte::with()` by `row::SameShape` — the same one comparison `SetOp` requires of `UNION`
branches, against the `RowCons` chain `with!{}` declares as `CteShape::Row`.

A set operation's `ORDER BY` takes an ordinal, because the branches' scopes are gone by
then — but the ordinal is `row::Field`'s index, which `scope::Position` reads, so
`SetOp::order_by_column` names a column and the index does the counting. The one output
shape with no key to name — a single un-tupled column, marked `select::SingleColumn` — takes
`.order_by(dir)` and counts to 1 itself. No public API takes a position: `OrdinalKey` is
private, since a position a caller writes is one nothing can check.

### Execution layer (`crates/qbrs-sqlx`)

Execution is bolted on via extension traits implemented only for `Postgres`-dialect
builders, plus `DecodeRow` impls for row decoding. The traits are cut by what a statement
*produces*, not by which builder it came from: `load`/`load_one` cover `SELECT`,
`RETURNING`, `DynSelect` and `SetOp` alike; `execute` is rows-affected DML; `count` is a
total; `PreparedExt`/`PreparedCountExt` are those with the `Params` that arrive at the call.

Each terminal is split in two: a *carrier* trait that says what a builder renders to
(`RowQuery`, `CountQuery`, `WriteStatement`, `PreparedQuery`, `PreparedTotal`) and carries
the `on_unimplemented` message, and an *extension* trait (`LoadExt`, `CountExt`,
`ExecuteExt`, `PreparedExt`, `PreparedCountExt`) whose method carries the bound. The
extension traits are implemented for every builder, dialect and all, so a terminal always
resolves and the carrier reports why it can't be satisfied — with the bound on the impl
instead, `.load()` on the wrong builder is a method-resolution failure that names five
unsatisfied bounds, or worse, suggests `Iterator`. They are not blanket impls:
`load`/`count`/`execute` are names other traits in a caller's scope have too. A new builder
therefore needs its empty `LoadExt`/`CountExt`/`ExecuteExt` impls plus the carrier impl that
is actually true of it. All methods are generic over
`sqlx::PgExecutor`, which is why `&PgPool` and `&mut *tx` both work with no separate
transactional API. Everything returns `qbrs_sqlx::Result<T>`; keep `UnresolvedPlaceholder`
(a qbrs-level misuse) distinct from the `Sqlx` variant rather than collapsing them.
`Value` has typed `NullX` variants specifically so NULLs bind with a declared wire type.
`DecodeRow` is what a caller's own generic signature names, so it is in the prelude.

## Conventions

- **A comment is a design smell, not a deliverable.** Reaching for one usually means an
  invariant is being kept by discipline that a type could have kept instead. Fix the
  structure first: give the concept a name (`Fragment`, `WindowFunc`), make the only
  constructor the correct one, split a state into two types rather than a flag. Write the
  comment only for what genuinely cannot be encoded — a language-level coherence rule, an
  external system's behavior — and then keep it to a sentence or two, stating the rule that
  holds now, never the history of how it was found. Do not narrate rejected alternatives or
  quote compiler error codes.
- **Deferred, not half-supported.** `WITH RECURSIVE`, aggregate window functions,
  CTE-referencing-CTE, row locking, a subquery in an expression position, and
  relations/eager-loading are explicitly out of scope and documented as "Known limitations"
  in the relevant module doc comment. Follow that pattern: state the
  limitation and why, don't silently fall back to `sql!{}`.
- **Adding a SQL feature** normally touches: an `ExprKind`/builder addition in `core`, a
  render arm, a `*_smoke.rs` test in `crates/core/tests` asserting the exact SQL string, a
  dialect capability trait if it isn't universal, a numbered example under `examples/examples/`
  (plus its row in `examples/README.md`), and the status/feature lists in the root `README.md`.
  A new *selectable* also needs a `select::RowField` impl (deciding its row key) and a
  matching `qbrs_sqlx::DecodeRow` impl if it decodes to a type nothing else does.
- Core tests are pure string-rendering assertions with `#[test]`. Anything a database has to
  agree with runs against one: Postgres in `qbrs-sqlx`, every rendered shape in
  `tests/dialect-exec` against SQLite, both with `#[tokio::test]`. Test names are full sentences
  (`right_join_flips_previously_joined_tables_to_nullable`).
- `tests/compile-bench/trybuild-drafts/*.draft` are deliberately-broken snippets kept out of
  the build; copy one into `tests/compile-bench/examples/` to eyeball the
  `#[diagnostic::on_unimplemented]` messages after changing them.
- CI pins third-party GitHub Actions to a full commit SHA; `actions/*` track release tags.
