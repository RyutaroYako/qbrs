//! A `JSON`/`JSONB` column, declared as `serde_json::Value` and carried
//! whole: it binds and it decodes, whichever of Postgres's two JSON types
//! the column is. Known limitations: the marker is `jsonb`'s, so comparing
//! or ordering by a `json` column compiles and is then rejected by the
//! server, which has those operators for `jsonb` alone; and the operators
//! that look *inside* a document (`->`, `->>`, `@>`) are not built — they
//! go through `sql!{}`, as the last query shows.
//! Run: `cargo run -p qbrs-examples --example 28_json`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;
use serde_json::json;

#[derive(Table)]
#[table(name = "suppressions")]
#[allow(dead_code)]
struct Suppressions {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    content: serde_json::Value,
    notes: Option<serde_json::Value>,
}

/// A plain domain struct, filled by field name like any other row.
#[derive(qbrs::FromRow, Debug)]
struct Suppression {
    email: String,
    content: serde_json::Value,
    notes: Option<serde_json::Value>,
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;

    sqlx::query("DROP TABLE IF EXISTS suppressions")
        .execute(&pool)
        .await
        .expect("drop suppressions");
    sqlx::query(
        "CREATE TABLE suppressions (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            email TEXT NOT NULL,
            content JSONB NOT NULL,
            notes JSON
        )",
    )
    .execute(&pool)
    .await
    .expect("create suppressions");

    insert(suppressions::Table)
        .values(
            SuppressionsInsert::builder()
                .email("ada@example.com")
                .content(json!({ "reason": "bounce", "codes": [550, 551] }))
                .notes(Some(json!({ "by": "ops" })))
                .build(),
        )
        // An omitted nullable JSON column is a NULL of that type, which is
        // a different thing from a document that *is* the JSON `null`.
        .values(
            SuppressionsInsert::builder()
                .email("dan@example.com")
                .content(json!(null))
                .build(),
        )
        .execute(&pool)
        .await
        .expect("seed the suppressions");

    let rows: Vec<Suppression> = select(suppressions::All)
        .from(suppressions::Table)
        .order_by(suppressions::id.asc())
        .load(&pool)
        .await
        .expect("read the suppressions back")
        .into_structs();
    for row in &rows {
        println!("{}: {} (notes {:?})", row.email, row.content, row.notes);
    }
    assert_eq!(rows[1].content, json!(null));
    assert_eq!(rows[1].notes, None);

    // Reading a key out is an operator, and those aren't built — the
    // escape hatch takes the column and the key as slots, so both are
    // still checked and bound. Postgres's `?` existence operator is the
    // one thing that cannot be written there, since a `?` in a `sql!{}`
    // text is a slot; `jsonb_exists(..)` is its function spelling.
    let bounced: Vec<String> = select(suppressions::email)
        .from(suppressions::Table)
        .filter(sql!(
            Bool,
            "jsonb_exists(?, ?)",
            suppressions::content,
            "reason"
        ))
        .filter(sql!(
            Bool,
            "(? ->> ?) = ?",
            suppressions::content,
            "reason",
            "bounce"
        ))
        .load(&pool)
        .await
        .expect("read a key through the escape hatch");
    println!("bounced: {bounced:?}");
    assert_eq!(bounced, vec!["ada@example.com".to_string()]);
}
