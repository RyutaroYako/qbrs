//! `JSON`/`JSONB` columns end to end: declared as `serde_json::Value`,
//! bound, and decoded back. Which of Postgres's two JSON types a column is
//! is the schema's business, so only a database says one marker carries
//! values to and from both. Only a database says what it does *not* carry,
//! which is why the `json` column here is never compared or ordered by:
//! `=` and `ORDER BY` are `jsonb`'s alone.
#![cfg(feature = "json")]

mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;
use serde_json::json;

#[derive(Table)]
#[table(name = "documents")]
#[allow(dead_code)]
struct Documents {
    #[column(primary_key, generated)]
    id: i64,
    /// `jsonb`, the one the issue named.
    body: serde_json::Value,
    /// `json`, the other spelling, and nullable.
    draft: Option<serde_json::Value>,
}

/// The DTO a handler returns, filled by field name.
#[derive(qbrs::FromRow, Debug, PartialEq)]
struct Doc {
    body: serde_json::Value,
    draft: Option<serde_json::Value>,
}

#[tokio::test]
async fn a_json_column_binds_and_decodes_as_the_document_it_holds() {
    let (pool, guard) = common::test_pool("qbrs_json").await;

    sqlx::query("DROP TABLE IF EXISTS documents")
        .execute(&pool)
        .await
        .expect("drop documents");
    sqlx::query(
        "CREATE TABLE documents (
            id BIGSERIAL PRIMARY KEY,
            body JSONB NOT NULL,
            draft JSON
        )",
    )
    .execute(&pool)
    .await
    .expect("create documents");

    let body = json!({ "kind": "suppression", "codes": [1, 2, 3], "note": null });
    let id: i64 = qbrs::insert::insert(documents::Table)
        .values(
            DocumentsInsert::builder()
                .body(body.clone())
                .draft(Some(json!({ "wip": true })))
                .build(),
        )
        .returning(documents::id)
        .load_one(&pool)
        .await
        .expect("insert a document")
        .expect("one row");

    // An omitted nullable JSON column is a NULL of that type, not an
    // untyped one, and stays apart from a JSON `null` inside a document.
    qbrs::insert::insert(documents::Table)
        .values(
            DocumentsInsert::builder()
                .body(json!(null))
                .draft(None::<serde_json::Value>)
                .build(),
        )
        .execute(&pool)
        .await
        .expect("insert a JSON null beside a SQL NULL");

    let docs: Vec<Doc> = select(documents::All)
        .from(documents::Table)
        .order_by(documents::id.asc())
        .load(&pool)
        .await
        .expect("read the documents back")
        .into_structs();
    assert_eq!(
        docs,
        vec![
            Doc {
                body: body.clone(),
                draft: Some(json!({ "wip": true })),
            },
            Doc {
                body: json!(null),
                draft: None,
            },
        ]
    );

    // `UPDATE .. SET <json column> = <document>` replaces the whole value.
    let changed = qbrs::update::update(documents::Table)
        .set(
            qbrs::update::Assignments::from_row(DocumentsUpdate {
                body: Some(json!({ "kind": "allow" })),
                ..Default::default()
            })
            .expect("body is set"),
        )
        .filter(documents::id.eq(id))
        .execute(&pool)
        .await
        .expect("update the document");
    assert_eq!(changed, 1);
    let replaced: serde_json::Value = select(documents::body)
        .from(documents::Table)
        .filter(documents::id.eq(id))
        .load_one(&pool)
        .await
        .expect("read the updated document")
        .expect("one row");
    assert_eq!(replaced, json!({ "kind": "allow" }));

    // Looking *inside* a document is an operator, and those are deferred.
    // The escape hatch takes the column and the value as slots, so both
    // are still checked and bound. Postgres's `?` existence operator is
    // the one that cannot be written there, since a `?` in a `sql!{}` text
    // is a slot; its function spelling is what `raw.rs` points at.
    let kinds: Vec<String> = select(qbrs::sql!(
        qbrs::expr::Text,
        "(? ->> ?)",
        documents::body,
        "kind"
    ))
    .from(documents::Table)
    .filter(qbrs::sql!(
        qbrs::expr::Bool,
        "jsonb_exists(?, ?)",
        documents::body,
        "kind"
    ))
    .load(&pool)
    .await
    .expect("read a key through the escape hatch");
    assert_eq!(kinds, vec!["allow".to_string()]);

    common::shutdown(pool, guard).await;
}
