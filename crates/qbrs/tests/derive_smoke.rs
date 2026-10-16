use qbrs::Table;
use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::select::{OrderExt, Predicate, Select, Selection, predicate, select};
use qbrs::statement::Statement;
use qbrs::update::Assignments;
use std::marker::PhantomData;

fn output_of<D, Scope, Sel, Idx>(_: &Select<D, Scope, Sel>, _: D) -> PhantomData<Sel::Output>
where
    Sel: Selection<Scope, Idx>,
{
    PhantomData
}

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
    #[column(default)]
    nickname: Option<String>,
    #[column(default)]
    active: bool,
}

#[derive(Table)]
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

fn rendered_user_insert(row: UsersInsert) -> (String, Vec<qbrs::expr::Value>) {
    qbrs::insert::insert(users::Table)
        .values(row)
        .to_sql(Postgres)
}

#[test]
fn an_omitted_nullable_defaulted_column_inserts_its_default() {
    let (sql, _) = rendered_user_insert(
        UsersInsert::builder()
            .email("a@example.com")
            .display_name("Ada")
            .build(),
    );
    assert!(sql.ends_with("VALUES ($1, $2, DEFAULT, DEFAULT)"), "{sql}");
}

#[test]
fn a_none_from_a_request_given_to_a_nullable_defaulted_column_inserts_its_default() {
    let absent: Option<String> = None;
    let (sql, _) = rendered_user_insert(
        UsersInsert::builder()
            .email("a@example.com")
            .display_name("Ada")
            .nickname(absent)
            .build(),
    );
    assert!(sql.ends_with("VALUES ($1, $2, DEFAULT, DEFAULT)"), "{sql}");
}

#[test]
fn the_null_setter_of_a_nullable_defaulted_column_inserts_an_explicit_null() {
    let (sql, params) = rendered_user_insert(
        UsersInsert::builder()
            .email("a@example.com")
            // A NULL `display_name` would share `$2` with the NULL under test,
            // so dropping this value would hide the explicit NULL behind it.
            .display_name("Ada")
            .nickname_null()
            .build(),
    );
    assert!(sql.ends_with("VALUES ($1, $2, $3, DEFAULT)"), "{sql}");
    assert_eq!(params[2], qbrs::expr::Value::NullText);
}

#[test]
fn two_columns_set_to_the_same_null_share_one_parameter() {
    let (sql, params) = rendered_user_insert(
        UsersInsert::builder()
            .email("a@example.com")
            .display_name(None::<String>)
            .nickname_null()
            .build(),
    );
    assert!(sql.ends_with("VALUES ($1, $2, $2, DEFAULT)"), "{sql}");
    assert_eq!(
        params,
        vec![
            qbrs::expr::Value::Text("a@example.com".into()),
            qbrs::expr::Value::NullText
        ]
    );
}

#[derive(Table)]
#[table(name = "feeds")]
#[allow(dead_code)]
struct Feeds {
    #[column(primary_key, generated)]
    id: i64,
    topics: Vec<String>,
    weights: Vec<i32>,
    seen: Vec<i64>,
    raw: Vec<u8>,
    notify: Option<Vec<String>>,
}

#[test]
fn an_array_column_compared_with_eq_binds_the_whole_array_as_one_parameter() {
    let (sql, params) = select((feeds::id, feeds::topics, feeds::notify))
        .from(feeds::Table)
        .filter(feeds::topics.eq(vec!["rust".to_string(), "sql".to_string()]))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        r#"SELECT "feeds"."id", "feeds"."topics", "feeds"."notify" FROM "feeds" WHERE ("feeds"."topics" = $1)"#
    );
    assert_eq!(
        params,
        vec![qbrs::expr::Value::TextArray(vec![
            "rust".to_string(),
            "sql".to_string()
        ])]
    );
}

#[test]
fn a_vec_u8_column_binds_as_bytes_rather_than_as_an_array() {
    let (sql, params) = select((feeds::id,))
        .from(feeds::Table)
        .filter(feeds::raw.eq(vec![1u8, 2]))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        r#"SELECT "feeds"."id" FROM "feeds" WHERE ("feeds"."raw" = $1)"#
    );
    assert_eq!(params, vec![qbrs::expr::Value::Bytes(vec![1, 2])]);
}

#[test]
fn eq_any_tests_a_value_against_a_plain_a_negated_or_a_nullable_array_column() {
    let (sql, params) = select((feeds::id,))
        .from(feeds::Table)
        .filter("rust".to_string().eq_any(feeds::topics))
        .filter(!1i32.eq_any(feeds::weights))
        .filter("nightly".to_string().eq_any(feeds::notify))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        r#"SELECT "feeds"."id" FROM "feeds" WHERE ($1 = ANY("feeds"."topics")) AND (NOT ($2 = ANY("feeds"."weights"))) AND ($3 = ANY("feeds"."notify"))"#
    );
    assert_eq!(
        params,
        vec![
            qbrs::expr::Value::Text("rust".to_string()),
            qbrs::expr::Value::I32(1),
            qbrs::expr::Value::Text("nightly".to_string()),
        ]
    );
}

#[test]
fn eq_any_binds_a_value_array_as_one_parameter_rather_than_one_per_element() {
    let (sql, params) = select((feeds::id,))
        .from(feeds::Table)
        .filter(feeds::id.eq_any(vec![1i64, 2, 3]))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        r#"SELECT "feeds"."id" FROM "feeds" WHERE ("feeds"."id" = ANY($1))"#
    );
    assert_eq!(params, vec![qbrs::expr::Value::BigIntArray(vec![1, 2, 3])]);
}

#[test]
fn an_omitted_nullable_text_array_binds_a_null_of_the_text_array_type() {
    let (_, params) = qbrs::insert::insert(feeds::Table)
        .values(
            FeedsInsert::builder()
                .topics(vec!["rust".to_string()])
                .weights(vec![1i32])
                .seen(vec![1i64])
                .raw(vec![0u8])
                .build(),
        )
        .to_sql(Postgres);
    assert!(
        params.contains(&qbrs::expr::Value::NullTextArray),
        "{params:?}"
    );
}

#[test]
fn insert_select_names_only_the_non_generated_columns_and_numbers_returning_after_the_body() {
    let source = select((
        feeds::topics,
        feeds::weights,
        feeds::seen,
        feeds::raw,
        feeds::notify,
    ))
    .from(feeds::Table)
    .filter(feeds::id.gt(10i64));
    let (sql, params) = qbrs::insert::insert(feeds::Table)
        .select(&source)
        .returning(qbrs::sql!(qbrs::expr::BigInt, "(? + ?)", feeds::id, 1i64))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "feeds" ("topics", "weights", "seen", "raw", "notify") SELECT "feeds"."topics", "feeds"."weights", "feeds"."seen", "feeds"."raw", "feeds"."notify" FROM "feeds" WHERE ("feeds"."id" > $1) RETURNING (("feeds"."id" + $2))"#
    );
    assert_eq!(
        params,
        vec![qbrs::expr::Value::I64(10), qbrs::expr::Value::I64(1)]
    );
}

#[cfg(feature = "json")]
#[derive(Table)]
#[table(name = "documents")]
#[allow(dead_code)]
struct Documents {
    #[column(primary_key, generated)]
    id: i64,
    body: serde_json::Value,
    draft: Option<serde_json::Value>,
}

#[cfg(feature = "json")]
#[test]
fn a_json_column_binds_as_one_parameter_and_an_omitted_nullable_one_as_a_json_null() {
    let body = serde_json::json!({ "kind": "suppression" });
    let (sql, params) = qbrs::insert::insert(documents::Table)
        .values(DocumentsInsert::builder().body(body.clone()).build())
        .to_sql(Postgres);
    assert_eq!(
        sql,
        r#"INSERT INTO "documents" ("body", "draft") VALUES ($1, $2)"#
    );
    assert_eq!(
        params,
        vec![qbrs::expr::Value::Json(body), qbrs::expr::Value::NullJson]
    );
}

#[test]
fn a_left_join_select_renders_where_order_by_and_limit_after_the_join() {
    let (sql, params) = select((users::id, users::display_name, orders::total))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::active.eq(true))
        .order_by(users::id.desc())
        .limit(5)
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "SELECT \"users\".\"id\", \"users\".\"display_name\", \"orders\".\"total\" \
         FROM \"users\" LEFT JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") \
         WHERE (\"users\".\"active\" = $1) ORDER BY \"users\".\"id\" DESC LIMIT 5"
    );
    assert_eq!(params, vec![qbrs::expr::Value::Bool(true)]);
}

#[test]
fn the_derived_insert_builder_binds_the_columns_it_sets_and_defaults_the_rest() {
    let (sql, params) = qbrs::insert::insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("a@example.com")
                .display_name("Ada")
                .build(),
        )
        .returning(users::id)
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"nickname\", \"active\") \
         VALUES ($1, $2, DEFAULT, DEFAULT) RETURNING \"users\".\"id\""
    );
    assert_eq!(
        params,
        vec![
            qbrs::expr::Value::Text("a@example.com".into()),
            qbrs::expr::Value::Text("Ada".into()),
        ]
    );
}

#[test]
fn an_update_from_a_row_sets_only_the_fields_the_row_touches() {
    let (sql, params) = qbrs::update::update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("New Name".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
        )
        .filter(users::id.eq(1i64))
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"display_name\" = $1 WHERE (\"users\".\"id\" = $2)"
    );
    assert_eq!(
        params,
        vec![
            qbrs::expr::Value::Text("New Name".into()),
            qbrs::expr::Value::I64(1),
        ]
    );
}

#[derive(Table)]
#[table(name = "flags")]
#[allow(dead_code)]
struct Flags {
    #[column(primary_key, generated)]
    id: i64,
    opted_in: Option<bool>,
}

#[test]
fn a_nullable_boolean_column_is_a_condition_on_its_own() {
    let (sql, params) = select((flags::id,))
        .from(flags::Table)
        .filter(flags::opted_in)
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "SELECT \"flags\".\"id\" FROM \"flags\" WHERE \"flags\".\"opted_in\""
    );
    assert!(params.is_empty());
}

#[test]
fn assigning_a_column_twice_keeps_the_last_assignment() {
    let (sql, params) = qbrs::update::update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("From the request".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
        )
        .set_to(users::display_name, "Computed")
        .to_sql(Postgres);

    assert_eq!(sql, "UPDATE \"users\" SET \"display_name\" = $1");
    assert_eq!(params, vec![qbrs::expr::Value::Text("Computed".into())]);
}

#[test]
fn set_to_a_null_binds_a_null_of_the_column_type() {
    let (sql, params) = qbrs::update::update(users::Table)
        .set_to(users::display_name, qbrs::expr::null::<qbrs::expr::Text>())
        .filter(users::id.eq(1i64))
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"display_name\" = $1 WHERE (\"users\".\"id\" = $2)"
    );
    assert_eq!(
        params,
        vec![qbrs::expr::Value::NullText, qbrs::expr::Value::I64(1)]
    );
}

#[test]
fn an_update_builder_leaves_an_absent_request_field_untouched_rather_than_setting_null() {
    let absent: Option<String> = None;
    let patch = UsersUpdate::builder()
        .display_name(absent)
        .nickname("Ada")
        .build();

    let (sql, params) = qbrs::update::update(users::Table)
        .set(Assignments::from_row(patch).expect("nickname is set"))
        .to_sql(Postgres);
    assert_eq!(sql, "UPDATE \"users\" SET \"nickname\" = $1");
    assert_eq!(params, vec![qbrs::expr::Value::Text("Ada".into())]);
}

#[test]
fn an_update_builder_sets_null_through_its_null_setter() {
    let cleared = UsersUpdate::builder().display_name_null().build();
    let (sql, params) = qbrs::update::update(users::Table)
        .set(Assignments::from_row(cleared).expect("display_name is set"))
        .to_sql(Postgres);
    assert_eq!(sql, "UPDATE \"users\" SET \"display_name\" = $1");
    assert_eq!(params, vec![qbrs::expr::Value::NullText]);
}

#[test]
fn all_row_is_the_row_type_select_all_decodes_to() {
    let _: PhantomData<users::AllRow> = output_of(&select(users::All).from(users::Table), Postgres);
}

#[test]
fn an_exists_discharged_into_a_predicate_sits_in_an_or_beside_a_comparison() {
    let base = select((users::email,)).from(users::Table);
    let has_flag = base
        .correlated(flags::Table, (flags::id,))
        .filter(flags::id.eq(users::id));

    let (sql, params) = base
        .clone()
        .filter(Predicate::any_of([
            predicate(users::active.eq(false)),
            predicate(has_flag.exists()),
        ]))
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "SELECT \"users\".\"email\" FROM \"users\" WHERE ((\"users\".\"active\" = $1) OR (EXISTS (SELECT \"flags\".\"id\" FROM \"flags\" WHERE (\"flags\".\"id\" = \"users\".\"id\"))))"
    );
    assert_eq!(params, vec![qbrs::expr::Value::Bool(false)]);
}

#[test]
fn an_update_builds_a_correlated_exists_over_its_own_table_without_a_select() {
    let statement = qbrs::update::update(users::Table).set_to(users::active, false);
    let has_flag = statement
        .correlated(flags::Table, (flags::id,))
        .filter(flags::id.eq(users::id));

    let (sql, params) = statement.filter(has_flag.exists()).to_sql(Postgres);
    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"active\" = $1 WHERE (EXISTS (SELECT \"flags\".\"id\" FROM \"flags\" WHERE (\"flags\".\"id\" = \"users\".\"id\")))"
    );
    assert_eq!(params, vec![qbrs::expr::Value::Bool(false)]);
}

#[test]
fn not_negates_a_sql_fragment_and_a_boolean_column_alike() {
    let (sql, _) = select((users::id,))
        .from(users::Table)
        .filter(!qbrs::sql!(
            qbrs::expr::Bool,
            "? IS NULL",
            users::display_name
        ))
        .filter(!users::active)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE (NOT (\"users\".\"display_name\" IS NULL)) AND (NOT \"users\".\"active\")"
    );
}

#[derive(Table)]
#[table(name = "events")]
#[allow(dead_code)]
struct Events {
    #[column(primary_key, generated)]
    id: i64,
    r#type: String,
}

#[test]
fn a_column_named_by_a_rust_keyword_renders_without_its_raw_identifier_prefix() {
    let (sql, params) = select((events::id, events::r#type))
        .from(events::Table)
        .filter(events::r#type.eq("signup"))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"events\".\"id\", \"events\".\"type\" FROM \"events\" WHERE (\"events\".\"type\" = $1)"
    );
    assert_eq!(params, vec![qbrs::expr::Value::Text("signup".into())]);

    let (insert_sql, _) = qbrs::insert::insert(events::Table)
        .values(EventsInsert::builder().r#type("signup").build())
        .to_sql(Postgres);
    assert_eq!(insert_sql, "INSERT INTO \"events\" (\"type\") VALUES ($1)");
}

#[derive(Table)]
#[table(name = "analytics.events")]
#[allow(dead_code)]
struct Metrics {
    #[column(primary_key, generated)]
    id: i64,
    name: String,
}

#[test]
fn a_schema_qualified_table_name_is_quoted_as_two_identifiers() {
    let (sql, _) = select((metrics::name,))
        .from(metrics::Table)
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"analytics\".\"events\".\"name\" FROM \"analytics\".\"events\""
    );
}

#[derive(Table)]
#[table(name = "import_runs")]
#[allow(dead_code)]
struct ImportRun {
    #[column(primary_key, generated)]
    id: i64,
    #[column(generated)]
    started_at: i64,
}

#[test]
fn a_table_whose_every_column_is_generated_inserts_default_values() {
    let (sql, params) = qbrs::insert::insert(import_run::Table)
        .values(ImportRunInsert::builder().build())
        .to_sql(Postgres);
    assert_eq!(sql, "INSERT INTO \"import_runs\" DEFAULT VALUES");
    assert!(params.is_empty());
}

#[test]
fn a_table_whose_every_column_is_generated_inserts_empty_values_on_mysql() {
    let (mysql, _) = qbrs::insert::insert(import_run::Table)
        .values(ImportRunInsert::builder().build())
        .to_sql(qbrs::dialect::MySql);
    assert_eq!(mysql, "INSERT INTO `import_runs` () VALUES ()");
}

#[test]
fn an_assignments_list_over_a_derived_schema_is_clone_and_debug() {
    let sets = Assignments::set_to(users::email, "a@example.com");
    let (sql, _) = qbrs::update::update(users::Table)
        .set(sets.clone())
        .to_sql(Postgres);
    assert_eq!(sql, "UPDATE \"users\" SET \"email\" = $1");
    assert!(format!("{sets:?}").starts_with("Assignments"));
}

#[cfg(feature = "uuid")]
mod uuid_from_the_prelude {
    use qbrs::prelude::*;

    #[derive(Table)]
    #[table(name = "tokens")]
    #[allow(dead_code)]
    struct Tokens {
        #[column(primary_key)]
        id: Uuid,
    }

    #[test]
    fn the_prelude_alone_supplies_a_uuid_that_is_both_the_marker_and_the_uuid_crate_type() {
        let id = Uuid::nil();
        let (sql, params) = select((tokens::id,))
            .from(tokens::Table)
            .filter(tokens::id.eq(id))
            .to_sql(Postgres);
        assert_eq!(
            sql,
            "SELECT \"tokens\".\"id\" FROM \"tokens\" WHERE (\"tokens\".\"id\" = $1)"
        );
        assert_eq!(params, vec![qbrs::expr::Value::Uuid(id)]);
    }
}

#[cfg(feature = "uuid")]
mod uuid_marker {
    use qbrs::prelude::*;
    use uuid::Uuid;

    #[derive(Table)]
    #[table(name = "sessions")]
    #[allow(dead_code)]
    struct Sessions {
        #[column(primary_key)]
        id: Uuid,
        replaces: Option<Uuid>,
    }

    qbrs::with! {
        struct live { id: Uuid }
    }

    #[test]
    fn a_uuid_imported_beside_the_prelude_glob_is_the_derive_and_sql_marker() {
        let id = Uuid::nil();
        let (sql, params) = select((sessions::id,))
            .from(sessions::Table)
            .filter(sessions::id.eq(qbrs::sql!(Uuid, "?", id)))
            .filter(sessions::replaces.is_null())
            .to_sql(Postgres);
        assert_eq!(
            sql,
            "SELECT \"sessions\".\"id\" FROM \"sessions\" \
             WHERE (\"sessions\".\"id\" = ($1)) AND (\"sessions\".\"replaces\" IS NULL)"
        );
        assert_eq!(params, vec![qbrs::expr::Value::Uuid(id)]);
    }

    #[test]
    fn a_uuid_imported_beside_the_prelude_glob_is_the_with_marker_a_derived_column_fills() {
        let bound = select((sessions::id,)).from(sessions::Table);
        let (with_sql, _) = select(live::id)
            .from(qbrs::cte::with(live::Table, &bound))
            .to_sql(Postgres);
        assert!(
            with_sql.starts_with("WITH \"live\" (\"id\") AS ("),
            "{with_sql}"
        );
    }
}
