#![cfg(feature = "json")]

mod common;

use qbrs::prelude::*;
use qbrs_sqlx::prelude::*;
use serde_json::json;
use sqlx::PgPool;

#[derive(Table)]
#[table(name = "documents")]
#[allow(dead_code)]
struct Documents {
    #[column(primary_key, generated)]
    id: i64,
    body: serde_json::Value,
    // Comparing or ordering by `draft` fails in Postgres: it is a `json`
    // column, and `=` and `ORDER BY` exist only for `jsonb`.
    draft: Option<serde_json::Value>,
}

#[derive(qbrs::FromRow, Debug, PartialEq)]
struct Doc {
    body: serde_json::Value,
    draft: Option<serde_json::Value>,
}

#[tokio::test]
async fn jsonb_and_json_columns_bind_and_decode_as_the_documents_they_hold() {
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

    a_sql_null_json_column_reads_back_apart_from_a_json_null_document(&pool, &body).await;
    an_update_replaces_a_json_document_whole(&pool, id).await;
    a_sql_fragment_reads_inside_a_document_with_the_column_and_key_bound_as_slots(&pool).await;

    common::shutdown(pool, guard).await;
}

async fn a_sql_null_json_column_reads_back_apart_from_a_json_null_document(
    pool: &PgPool,
    first_body: &serde_json::Value,
) {
    qbrs::insert::insert(documents::Table)
        .values(
            DocumentsInsert::builder()
                .body(json!(null))
                .draft(None::<serde_json::Value>)
                .build(),
        )
        .execute(pool)
        .await
        .expect("insert a JSON null beside a SQL NULL");

    let docs: Vec<Doc> = select(documents::All)
        .from(documents::Table)
        .order_by(documents::id.asc())
        .load(pool)
        .await
        .expect("read the documents back")
        .into_structs();
    assert_eq!(
        docs,
        vec![
            Doc {
                body: first_body.clone(),
                draft: Some(json!({ "wip": true })),
            },
            Doc {
                body: json!(null),
                draft: None,
            },
        ]
    );
}

async fn an_update_replaces_a_json_document_whole(pool: &PgPool, id: i64) {
    let changed = qbrs::update::update(documents::Table)
        .set(
            qbrs::update::Assignments::from_row(DocumentsUpdate {
                body: Some(json!({ "kind": "allow" })),
                ..Default::default()
            })
            .expect("body is set"),
        )
        .filter(documents::id.eq(id))
        .execute(pool)
        .await
        .expect("update the document");
    assert_eq!(changed, 1);
    let replaced: serde_json::Value = select(documents::body)
        .from(documents::Table)
        .filter(documents::id.eq(id))
        .load_one(pool)
        .await
        .expect("read the updated document")
        .expect("one row");
    assert_eq!(replaced, json!({ "kind": "allow" }));
}

async fn a_sql_fragment_reads_inside_a_document_with_the_column_and_key_bound_as_slots(
    pool: &PgPool,
) {
    let kinds: Vec<String> = select(qbrs::sql!(
        qbrs::expr::Text,
        "(? ->> ?)",
        documents::body,
        "kind"
    ))
    .from(documents::Table)
    // Postgres's `body ? 'kind'` cannot be written in `sql!{}` text, where a
    // `?` is a slot, so the existence test takes its function spelling.
    .filter(qbrs::sql!(
        qbrs::expr::Bool,
        "jsonb_exists(?, ?)",
        documents::body,
        "kind"
    ))
    .load(pool)
    .await
    .expect("read a key through the escape hatch");
    assert_eq!(kinds, vec!["allow".to_string()]);
}
