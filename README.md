<p align="center">
  <img src="https://raw.githubusercontent.com/RyutaroYako/qbrs/main/assets/logo-dark.png" alt="qbrs" width="320">
</p>

<p align="center"><b>A Drizzle-flavored, type-safe SQL query builder for Rust.</b></p>

<p align="center">
<a href="https://crates.io/crates/qbrs"><img src="https://img.shields.io/crates/v/qbrs.svg" alt="crates.io"></a>
<a href="https://docs.rs/qbrs"><img src="https://docs.rs/qbrs/badge.svg" alt="docs.rs"></a>
<a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
<img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="Rust 1.88+">
</p>

<p align="center">
<a href="#quick-example">Quick example</a> ·
<a href="#why-qbrs">Why qbrs?</a> ·
<a href="#install">Install</a> ·
<a href="#whats-in-it">What's in it</a> ·
<a href="#known-limitations">Known limitations</a> ·
<a href="#status">Status</a>
</p>

---

qbrs checks column and join references at compile time without giving up
dynamic query composition — most builders make you pick one. See [Why
qbrs?](#why-qbrs) for how.

> [!WARNING]
> **Not production ready.** 0.1.0 is the first public release. The API will
> break between 0.x minors, only Postgres has an execution layer, and nothing
> here has been run against a real workload yet. Worth trying and filing
> issues against; not worth putting under something that matters.

> **Not an ORM.** qbrs builds and renders SQL with compile-time-checked
> column/join references; it doesn't do change-tracking, identity maps, or
> hide SQL behind an object graph. Want that? See
> [SeaORM](https://www.sea-ql.org/SeaORM/). Want raw SQL with compile-time
> type-checking instead of a builder? See
> [sqlx](https://github.com/launchbadge/sqlx).

## Quick example

```rust
#[derive(Table)]
#[table(name = "users")]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
    #[column(default)]
    active: bool,
}

#[derive(Table)]
#[table(name = "orders")]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

let rows = select((users::email, orders::total))
    .from(users::Table)
    .left_join(orders::Table, orders::user_id.eq(users::id))
    .order_by(users::id.asc())
    .load(&pool)
    .await?;

for row in &rows {
    println!("{} {:?}", row.email(), row.total()); // &String, &Option<i64>
}
```

```sql
-- rendered by qbrs:
SELECT "users"."email", "orders"."total"
FROM "users" LEFT JOIN "orders" ON ("orders"."user_id" = "users"."id")
ORDER BY "users"."id" ASC
```

`orders::total` is declared as a plain `i64` column, but because it's on the
far side of a `LEFT JOIN`, `row.total()` comes back as `&Option<i64>`
automatically. A row is read by the column value that selected it, not by
position, so adding a column to the selection moves nothing. Forget the join
and reference `orders::total` anyway, and it's a compile error:

```
error[E0277]: `orders::Table` is not available in this query's scope
  --> src/main.rs:23:22
   |
23 |       let rows = select((users::email, orders::total))
   |  ________________^
24 | |         .from(users::Table)
   | |____________________________^ add `.join(<table>, ..)` (or `.from(..)`)
   |                                for `orders::Table` before referencing
   |                                its columns here
```

## Why qbrs?

|                                                                                         |    qbrs     |                                            diesel                                            |       sea-query        |            sqlx            |
| --------------------------------------------------------------------------------------- | :---------: | :------------------------------------------------------------------------------------------: | :--------------------: | :------------------------: |
| Compile-time column/join validity checking                                              |     ✅      |                                              ✅                                              |    ❌ (runtime AST)    |    n/a — not a builder     |
| `NULL`-ability auto-derived from join kind                                              |     ✅      |                                  ❌ (manual `.nullable()`)                                   |           ❌           |            n/a             |
| Add predicates conditionally/in a loop, no escape hatch                                 |     ✅      |                                   ⚠️ needs `.into_boxed()`                                   | ✅ (dynamic by design) | ⚠️ drops to `QueryBuilder` |
| Table/column refs are plain values, not turbofish/closures                              |     ✅      |                                           partial                                            |           ✅           |            n/a             |
| Result rows keyed by column, not by position                                            |     ✅      |                                              ❌                                              |           ❌           |     ✅ (`query_as!`)      |
| Compile time at ~40+ joins (measured, see [`tests/compile-bench`](tests/compile-bench)) | linear, ~ms | [documented exponential blowup](https://github.com/diesel-rs/diesel/issues/3223) at ~7 joins |          n/a           |            n/a             |

The trick: whether a column reference makes sense given what's joined is a
compile-time question, checked once via a flat type-level list (see
[`crates/core/src/scope.rs`](crates/core/src/scope.rs)). How many predicates
you've added is a runtime question — a plain `Vec`, so `.filter()` can be
called conditionally or in a loop without changing the query's type. Most
builders conflate the two and need an escape hatch (`.$dynamic()`,
`.into_boxed()`) the moment a query gets built conditionally. qbrs needs one
too, but it's deliberately narrow — see [Known limitations](#known-limitations).

## Install

```toml
[dependencies]
qbrs = "0.3.0"
qbrs-sqlx = "0.3.0"                                                     # Postgres execution via sqlx
sqlx = { version = "0.9", features = ["runtime-tokio", "postgres"] }  # for `PgPool`
```

`qbrs-sqlx`'s methods take any `sqlx::PgExecutor`, so the `sqlx` version has
to be the one it is built against (0.9). Column types that need a crate to
decode to are features — `chrono`, `uuid`, `decimal`, `json` — and each has to be enabled on
**both** `qbrs` and `qbrs-sqlx`, which are separate `cfg`s over one `Value`:
enabling it on `qbrs` alone is a compile error where the value is decoded
and a `FeatureNotEnabled` where one is bound.

## What's in it

`SELECT`/`INSERT`/`UPDATE`/`DELETE` (`INSERT .. SELECT` included), every JOIN kind, `GROUP BY`/`HAVING`,
aggregates (`count`/`count_of`/`sum`/`min`/`max`/`avg`/`string_agg`),
`DISTINCT`, upsert (`ON CONFLICT`, partial unique indexes, `excluded(..)` and a conditional `DO UPDATE` included), `UNION`/`INTERSECT`/`EXCEPT`,
ranking window functions, non-recursive CTEs (Postgres data-modifying ones
included), correlated `EXISTS`,
a `SELECT` with no `FROM` (`now()`, `pg_try_advisory_lock($1)`),
Postgres array columns (`Vec<T>` as `TEXT[]`/`INTEGER[]`/`BIGINT[]`/`UUID[]`, with `= ANY(..)`),
`JSON`/`JSONB` columns (`serde_json::Value`),
`IN (SELECT ..)`/`NOT IN (SELECT ..)`, transactions, streaming
(`.stream(..)`), the `sql!{}` escape hatch, and typed prepared statements
(`prepare!{}`).

A schema is a `#[derive(Table)]` struct; `use qbrs::prelude::*;` and
`use qbrs_sqlx::prelude::*;` cover a query. Everything above has a runnable,
end-to-end example against a real Postgres — see
**[`examples/README.md`](examples/README.md)** for the index, and
[docs.rs/qbrs](https://docs.rs/qbrs) for the API.

Three things that aren't obvious from a signature:

- **Rows are keyed by column, not by position.** A tuple selection decodes to
  a `Row` read with `row.get(users::email)` or the generated `row.email()`;
  `#[derive(FromRow)]` fills a plain domain struct by field name, with no
  column path or table in it. `into_tuples()` recovers the positional view.
- **A statement with nothing in it has no SQL form.** An `*Update` whose every
  field is untouched, or an insert of zero rows, hands back
  `NothingToSet`/`NothingToInsert` rather than rendering broken SQL. These are
  the only fallible builder methods; everything downstream is infallible.
- **The dialect is part of a query's type** — capability gating happens while
  the query is built, not when it renders — but it's never a turbofish:
  `.load(&pool)` infers it from the executor, `.to_sql(Postgres)` takes it as
  a value.

## Known limitations

Deferred rather than half-supported, and documented in the relevant module:
`WITH RECURSIVE`, aggregates as window functions (`sum(x) OVER (..)`), a CTE
referencing another CTE, a data-modifying CTE anywhere but the top level
(Postgres refuses one inside an `EXISTS`/`IN` subquery or a set-operation
branch, and that is the server's error rather than the compiler's), row
locking (`FOR UPDATE`/`SKIP LOCKED`), a scalar
subquery in an expression position (`col = (SELECT max(x) ..)`), the array
operators (`@>`, `&&`, `array_append` — `= ANY(..)` is `.eq_any(..)`) and the array element
types beyond the four (`BOOLEAN[]`, `DOUBLE PRECISION[]`, `TIMESTAMPTZ[]`,
`NUMERIC[]`, and any array whose elements can be NULL), the JSON operators
(`->`, `->>`, `@>`, `?`) and a `json` column's missing `=`/`ORDER BY` (the
marker is `jsonb`'s), `ON CONFLICT` or a
column subset on an `INSERT .. SELECT` (it fills every column the target
lets a statement write, and its source is a `Select` rather than a
`UNION` or a `DynSelect`), and relations/eager-loading. `sql!{}` doesn't reach the last two: it builds an
expression, not a statement suffix, and a `Select` isn't a slot value.
`IN (SELECT ..)`/`NOT IN (SELECT ..)` is covered by `Select::contains`/
`.not_contains`, which — like `EXISTS` — is dialect-pinned rather than a
plain expression.

Design constraints worth knowing before adopting:

- **Conditionally *joining* a table has no fully-static solution** — a single
  type can't mean "joined" in one branch and "not joined" in another.
  `.erase()` into `DynSelect` is the way out, and it's narrow: only the join
  skeleton is erased, and no further `.filter()`/`.join()` is offered on it.
  Conditional *filtering* needs none of this.
- **No table aliasing, so no literal `FROM "t" AS a JOIN "t" AS b`.** Two
  `#[derive(Table)]` structs must not share a `#[table(name = "..")]` — that
  compiles and then renders `FROM "t" JOIN "t"`, which the database refuses.
  A `with!{}` pseudo-table bound to a plain `select(..).from(t::Table)` gets
  the same result today: `with! { struct managers { id: Integer, name: Text } }`
  then `select((employees::name, managers::name)).from(employees::Table)
  .inner_join(cte::with(managers::Table,
  &select((employees::id, employees::name)).from(employees::Table)),
  managers::id.eq(employees::manager_id))` renders a `WITH` CTE joined back
  to the same table — correct, type-checked, and available now — rather
  than the bare-alias SQL shape. The declared column types are the body's:
  `Integer` because `employees::id` is an `i32`.
- **`GROUP BY` isn't related to the selection list.** Every non-aggregated
  selected column has to appear in `GROUP BY` (or be functionally
  dependent), and nothing here checks that — it's the selection-into-`GROUP
  BY` direction, the reverse of what a `row::Field` lookup can check (`GROUP
  BY` naming a column that isn't selected is perfectly valid SQL, so
  checking membership the other way round would be enforcing a rule that
  doesn't exist). An aggregate or window function in `WHERE` or `RETURNING`
  is accepted by the builder and rejected by the database, too. Aggregates
  take a bare column: `sum(price * qty)` and `count(DISTINCT x)` need
  `sql!{}`, as does an `ORDER BY` inside a `string_agg` — SQLite reached
  that only in 3.44, past the 3.39 this crate targets, and MySQL spells it
  elsewhere in the call. `string_agg`'s separator binds under Postgres and
  SQLite, which take it as an argument; MySQL's `SEPARATOR` takes a literal
  and rejects a parameter, so there it is written into the SQL, which is
  why it is a `&'static str` everywhere. MySQL also truncates the result at
  `group_concat_max_len` (1024 bytes by default) with a warning rather than
  an error.
- **`ORDER BY` has a checked and an unchecked form.** Plain `.order_by(..)`
  only checks scope membership, since a non-`DISTINCT` query may sort by any
  column in scope. `.order_by_selected(..)`/`.order_by_selection(..)` also
  check the sort key is in the selection — the same `row::Field` lookup
  `Row::get` uses — which is exactly what `SELECT DISTINCT` requires
  (Postgres rejects a sort key that isn't selected): pair `.distinct()` with
  one of these instead of plain `.order_by(..)` for a query that can't
  render SQL the database would reject. One thing they still don't catch:
  `.reselect(..)` after one of them keeps the `ORDER BY` it added, so swap
  the selection before sorting by it.
- **A computed expression's nullability isn't derived** the way a column's is.
  An expression whose type the builder inferred — a comparison, an `is_null`,
  a `LIKE` — says what it decodes to once, with `.decodes_as::<Bool>()`; a
  `sql!{}` fragment states its type in the macro.
- **A selection list holds at most 32 elements** (`<table>::All` counts as
  one, whatever the column count). Naming a row type in a signature takes a
  type alias long enough to trip `clippy::type_complexity`; inference covers
  everything that stays inside a function, and `<table>::AllRow` covers a
  stored `select(All)`.
- **One `label!` per scope** — it declares a `label` module, and a scope holds
  one. List every name that scope needs in the one invocation.
- **The derives expand to `::qbrs::` paths**, so depend on the `qbrs` facade
  rather than on `qbrs-core` + `qbrs-macros` directly.
- **A JSON column is a document, not a structure.** `serde_json::Value`
  binds and decodes whole. The marker is `jsonb`'s: a `json` column takes
  the same values back and forth, but only `jsonb` has an equality and an
  ordering operator, so `.eq(..)`/`.asc()`/`GROUP BY` on a `json` column is
  the server's error rather than the compiler's. The operators that look
  inside a document (`->`, `->>`, `@>`) are not built and go through
  `sql!{}`; Postgres's `?` existence operators are the ones that cannot,
  since a `?` there is a slot, so they are reached as `jsonb_exists(..)`,
  `jsonb_exists_any(..)` and `jsonb_exists_all(..)`.
- **An array column is a value, not a set.** `Vec<T>` binds and decodes as
  a Postgres array, and `=` compares two of them whole. `.eq_any(..)` asks
  the one question about an element — `x = ANY(arr)`, which is what `is_in`
  asks of a written-out list, of an array the database unnests. The array
  *operators* — `@>`, `&&`, `array_append` — are not built; they go through
  `sql!{}`, where the column and the value are still slots.
  MySQL and SQLite have no array type at all, and since an `Expr` carries no
  dialect there is nothing to gate on: an array reaches those two as a bind
  their driver refuses, and `= ANY(..)` as a statement they won't parse.
- **Every `?` in a `sql!{}` text is a slot**, with no escape for a literal one
  — MySQL and SQLite spell their bind parameters the same way. Its text must
  be a constant (a literal, a `const`, `concat!`, `include_str!`), so
  runtime-assembled text can never become SQL shape. One fragment reused
  across clauses of a statement — selected, grouped by, ordered by — renders
  as one expression, which is what Postgres's syntactic `GROUP BY` matching
  asks for; where its placeholders are numbered a repeated value is named
  again rather than bound again, so the rendered text depends on which of a
  statement's values are equal, as it already depends on how many rows an
  `INSERT` carries.

## Status

|                                                                             | Postgres |  MySQL  | SQLite  |
| --------------------------------------------------------------------------- | :------: | :-----: | :-----: |
| Query building & SQL rendering                                              |    ✅    |   ✅    |   ✅    |
| Rendered SQL executed in CI                                                 |    ✅    | not yet |   ✅    |
| Dialect capability gating (`RETURNING`, `ON CONFLICT`, `RIGHT`/`FULL JOIN`, data-modifying CTE) |    ✅    |   ✅    |   ✅    |
| Execution (via `qbrs-sqlx`)                                                 |    ✅    | not yet | not yet |
| Transactions (via `qbrs-sqlx`)                                              |    ✅    | not yet | not yet |
| Streaming (`.stream(..)`, via `qbrs-sqlx`)                                  |    ✅    | not yet | not yet |

MySQL is rendered and asserted as strings only, so its dialect differences are
caught only where someone thought to look. One known difference: `DEFAULT` in
an `INSERT ... VALUES` is Postgres and MySQL only — SQLite rejects it, which
makes `Defaultable::Default` unusable there.

## Development

- [`crates/core`](crates/core) — type-level machinery and SQL rendering. No
  I/O, no async, no driver.
- [`crates/macros`](crates/macros) — `#[derive(Table)]`, `#[derive(FromRow)]`,
  `label!`, `with!`.
- [`crates/qbrs`](crates/qbrs) — the facade; depend on this one.
- [`crates/qbrs-sqlx`](crates/qbrs-sqlx) — execution via `sqlx` (Postgres).
- [`examples`](examples) — runnable examples; [`tests/dialect-exec`](tests/dialect-exec)
  runs every rendered shape against SQLite; [`tests/compile-bench`](tests/compile-bench)
  backs the "linear at 40+ joins" claim.

```sh
cargo test --workspace --all-features
cargo nextest run --workspace --all-features   # all but the doctests, in parallel
```

No setup required: the real-DB tests start their own throwaway PostgreSQL
17.5, embedded via [`pglite-rs`](https://crates.io/crates/pglite-rs) — no
Docker and no service to launch. The engine is downloaded once, when the crate
is first built, and cached under `~/.cache/pglite-rs`; nothing is fetched while
a test runs. The first run also pays for an `initdb`, then caches the data
directory under `target/` for every later test and example to copy. Set
`DATABASE_URL` to run them against an external Postgres instead.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Contributions are licensed the same way unless stated otherwise.
