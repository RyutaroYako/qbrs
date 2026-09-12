use qbrs::Table;
use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::select::{OrderExt, Predicate, predicate, select};
use qbrs::statement::Statement;
use qbrs::update::Assignments;

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
    /// Nullable *and* defaulted: the three-state column.
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

#[test]
fn a_nullable_defaulted_column_says_its_three_states_apart() {
    // Omitted: the schema's default. `None`: the same, since that is what a
    // request field without a value means. `_null()`: an explicit NULL.
    let omitted = qbrs::insert::insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("a@example.com")
                .display_name("Ada")
                .build(),
        )
        .to_sql(Postgres)
        .0;
    assert!(
        omitted.ends_with("VALUES ($1, $2, DEFAULT, DEFAULT)"),
        "{omitted}"
    );

    let absent: Option<String> = None;
    let from_request = qbrs::insert::insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("a@example.com")
                .display_name("Ada")
                .nickname(absent)
                .build(),
        )
        .to_sql(Postgres)
        .0;
    assert!(
        from_request.ends_with("VALUES ($1, $2, DEFAULT, DEFAULT)"),
        "{from_request}"
    );

    let (explicit, params) = qbrs::insert::insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("a@example.com")
                .display_name("Ada")
                .nickname_null()
                .build(),
        )
        .to_sql(Postgres);
    assert!(
        explicit.ends_with("VALUES ($1, $2, $3, DEFAULT)"),
        "{explicit}"
    );
    assert_eq!(params[2], qbrs::expr::Value::NullText);
}

/// Two columns explicitly set to NULL are one value, and Postgres names a
/// parameter rather than rebinding it — so the row's third cell is the
/// second cell's parameter. Which is why the three-state test above gives
/// `display_name` a value: otherwise its NULL and `nickname`'s would be
/// the same `$2`, and the states it is checking would be indistinguishable
/// in the rendered SQL.
#[test]
fn two_columns_set_to_the_same_null_share_one_parameter() {
    let (sql, params) = qbrs::insert::insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("a@example.com")
                .display_name(None::<String>)
                .nickname_null()
                .build(),
        )
        .to_sql(Postgres);
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

/// An array is one bind parameter, not a rendered list: `IN (a, b)` and
/// `= ARRAY[a, b]` are different questions, and a `Vec<T>` column holds the
/// second. `Vec<u8>` stays `bytea`, which is what keeps the derive's `Vec`
/// arm a decision rather than a guess.
#[test]
fn an_array_column_binds_as_one_parameter_and_vec_u8_stays_bytes() {
    let (sql, params) = select((feeds::id, feeds::topics, feeds::notify))
        .from(feeds::Table)
        .filter(feeds::topics.eq(vec!["rust".to_string()]))
        .filter(feeds::raw.eq(vec![1u8, 2]))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        r#"SELECT "feeds"."id", "feeds"."topics", "feeds"."notify" FROM "feeds" WHERE ("feeds"."topics" = $1) AND ("feeds"."raw" = $2)"#
    );
    assert_eq!(
        params,
        vec![
            qbrs::expr::Value::TextArray(vec!["rust".to_string()]),
            qbrs::expr::Value::Bytes(vec![1, 2]),
        ]
    );
}

/// `x = ANY(arr)` asks the question `IN` asks of a written-out list, of an
/// array the database unnests — so the array is a column, which is what the
/// list form cannot be. `!` is "none of them", and a nullable array column
/// answers it too.
#[test]
fn a_value_is_tested_against_an_array_column_with_eq_any() {
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

/// The array can be a bound value too, which is the shape a request's own
/// list of ids arrives as — one parameter rather than one per element.
#[test]
fn eq_any_takes_a_bound_array_as_well_as_a_column() {
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

/// An omitted nullable array is a NULL of the array's own type, not an
/// untyped one — the same reason every other `NullX` variant exists.
#[test]
fn an_omitted_nullable_array_binds_a_typed_null() {
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

/// `INSERT INTO t (..) SELECT ..` names the columns the target lets a
/// statement write — `feeds::id` is generated, so it is the database's to
/// fill and naming it would be an error Postgres raises. A `RETURNING`
/// that binds is numbered after the body, which is the whole reason the
/// body is a `Fragment` rather than a rendered string.
#[test]
fn insert_select_names_the_writable_columns_and_numbers_returning_after_the_body() {
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

/// A JSON document is one bind parameter, opaque to the renderer, and an
/// omitted nullable one is a NULL of that type rather than an untyped one.
#[cfg(feature = "json")]
#[test]
fn a_json_column_binds_as_one_opaque_parameter() {
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
fn schema_module_and_select_builder_work_together() {
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
fn insert_uses_generated_new_and_setters() {
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
fn update_only_sends_touched_fields() {
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
fn set_to_assigns_a_typed_sql_null() {
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
fn an_update_builder_leaves_an_absent_request_field_untouched() {
    // The struct literal spells a nullable column's three states as
    // `Option<Option<T>>`; the builder takes what a request holds, so an
    // absent field can't turn into `SET display_name = NULL`.
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

    // The explicit NULL is its own call, as it is on the insert builder.
    let cleared = UsersUpdate::builder().display_name_null().build();
    let (sql, params) = qbrs::update::update(users::Table)
        .set(Assignments::from_row(cleared).expect("display_name is set"))
        .to_sql(Postgres);
    assert_eq!(sql, "UPDATE \"users\" SET \"display_name\" = $1");
    assert_eq!(params, vec![qbrs::expr::Value::NullText]);
}

#[test]
fn all_row_names_what_select_all_decodes_to() {
    fn take(row: users::AllRow) -> i64 {
        *row.get(users::id)
    }
    let _ = take;
}

#[test]
fn an_exists_composes_with_other_conditions_once_discharged() {
    // `Exists` is dialect-pinned, but discharging it doesn't give that up:
    // a `Predicate` carries the dialect too, so an `EXISTS` can sit in an
    // OR beside an ordinary comparison.
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
fn a_write_statement_builds_its_own_correlated_exists() {
    // The same `EXISTS` a `SELECT` builds, against the one-table scope an
    // `UPDATE` has — no throwaway `Select` to hang it on.
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
fn a_sql_fragment_negates_and_a_boolean_column_does_too() {
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
    /// A column whose SQL name is a Rust keyword: `r#` is the language's
    /// escape, not the database's, so it must not reach the SQL.
    r#type: String,
}

#[test]
fn a_column_named_by_a_rust_keyword_renders_its_sql_name() {
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
fn a_schema_qualified_table_is_two_identifiers() {
    // `analytics.events` is a table in a schema, not a table whose name has
    // a dot in it — quoting it whole asks for a relation nobody created.
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
fn a_table_with_nothing_to_insert_says_default_values() {
    // Every column is the database's to write, so the row names none — and
    // an empty column list is a syntax error in two of the three dialects.
    let (sql, params) = qbrs::insert::insert(import_run::Table)
        .values(ImportRunInsert::builder().build())
        .to_sql(Postgres);
    assert_eq!(sql, "INSERT INTO \"import_runs\" DEFAULT VALUES");
    assert!(params.is_empty());

    let (mysql, _) = qbrs::insert::insert(import_run::Table)
        .values(ImportRunInsert::builder().build())
        .to_sql(qbrs::dialect::MySql);
    assert_eq!(mysql, "INSERT INTO `import_runs` () VALUES ()");
}

#[test]
fn an_assignments_list_clones_and_prints() {
    // Its phantom table marker is a bare unit struct, so a derived `Clone`
    // would apply to no schema at all.
    let sets = Assignments::set_to(users::email, "a@example.com");
    let (sql, _) = qbrs::update::update(users::Table)
        .set(sets.clone())
        .to_sql(Postgres);
    assert_eq!(sql, "UPDATE \"users\" SET \"email\" = $1");
    assert!(format!("{sets:?}").starts_with("Assignments"));
}
