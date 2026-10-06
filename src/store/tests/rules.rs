use super::*;
use serde_json::json;

pub(crate) fn definition(name: &str, script: &str) -> Definition {
    Definition {
        name: name.into(),
        description: "Inspect matching events".into(),
        script: script.into(),
        template: "blank".into(),
        model: "openai/test-model".into(),
        variant: None,
        initial_prompt: "Inspect {{event.data.text}} from {{event.provider}}".into(),
        enabled: true,
        start_instance: true,
        focus_pane: true,
    }
}

#[test]
fn rule_pane_focus_defaults_on_and_preserves_explicit_background_selection() {
    let mut value =
        serde_json::to_value(definition("inspect", "fn matches(event) { true }")).unwrap();
    value.as_object_mut().unwrap().remove("focus_pane");
    let rule: Definition = serde_json::from_value(value.clone()).unwrap();
    assert!(rule.focus_pane);
    value["focus_pane"] = json!(false);
    let rule: Definition = serde_json::from_value(value).unwrap();
    assert!(!rule.focus_pane);
    assert_eq!(serde_json::to_value(rule).unwrap()["focus_pane"], false);
}

#[test]
fn optional_rule_variants_preserve_defaults_and_embedded_selection() {
    let mut value =
        serde_json::to_value(definition("sample", "fn matches(event) { true }")).unwrap();
    assert!(value.get("variant").is_none());
    let rule: Definition = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(rule.variant, None);
    assert_eq!(
        rule.session_launch().selector().as_deref(),
        Some("openai/test-model")
    );
    value["variant"] = json!("high");
    let rule: Definition = serde_json::from_value(value.clone()).unwrap();
    validate(&rule).unwrap();
    assert_eq!(
        rule.session_launch().selector().as_deref(),
        Some("openai/test-model#high")
    );
    value["model"] = json!("openai/test-model#fast");
    assert!(
        validate(&serde_json::from_value(value.clone()).unwrap())
            .unwrap_err()
            .contains("conflicts")
    );
    value.as_object_mut().unwrap().remove("variant");
    let rule: Definition = serde_json::from_value(value).unwrap();
    validate(&rule).unwrap();
    assert_eq!(
        rule.session_launch().selector().as_deref(),
        Some("openai/test-model#fast")
    );
}

#[test]
fn instance_names_preserve_acceptance_identity_within_length_limits() {
    assert_eq!(
        instance_name("jira-ticket-triage", 42, 1),
        "jira-ticket-triage-42-a1"
    );
    assert_eq!(
        instance_name("jira-ticket-triage", 42, 2),
        "jira-ticket-triage-42-a2"
    );
    let rule = "a".repeat(40);
    for id in [1, 2, i64::MAX] {
        let name = instance_name(&rule, i64::MAX, id);
        assert_eq!(name.len(), 40);
        super::super::environments::validate_instance_name(&name).unwrap();
        assert!(name.ends_with(&format!("-a{id}")), "{name}");
    }
}

#[test]
fn predicates_inspect_all_profiles_and_nested_metadata_without_host_capabilities() {
    for profile in ["message", "ticket", "generic", "system_event"] {
        let rule = definition(
            "sample",
            "fn matches(event) { event.metadata.route == \"work\" && event.data.count == 3 }",
        );
        let input =
            json!({"profile": profile, "data": {"count": 3}, "metadata": {"route": "work"}});
        assert!(matches(&rule, &input).unwrap());
    }
    for script in [
        "fn matches(event) { eval(\"true\") }",
        "import \"/etc/passwd\"; fn matches(event) { true }",
    ] {
        assert!(validate(&definition("sample", script)).is_err());
    }
    let runaway = definition("sample", "fn matches(event) { loop {} true }");
    assert!(matches(&runaway, &json!({})).is_err());
    assert!(
        matches(
            &definition("sample", "fn matches(event) { 42 }"),
            &json!({})
        )
        .is_err()
    );
}

#[test]
fn prompt_rendering_is_single_pass_and_reports_missing_fields() {
    let event =
        json!({"data": {"text": "{{event.secret}}"}, "metadata": {"nested": {"value": 42}}});
    assert_eq!(
        render_prompt(
            "{{event.data.text}} / {{event.metadata.nested.value}}",
            &event
        )
        .unwrap(),
        "{{event.secret}} / 42"
    );
    assert_eq!(
        render_prompt("{{event}}", &event).unwrap(),
        event.to_string()
    );
    assert_eq!(
        render_prompt("{{event.secret}}", &event).unwrap_err(),
        "missing prompt field: event.secret"
    );
    assert!(render_prompt("{{event.data.text", &event).is_err());
    assert_eq!(
        render_prompt("{{event.metadata}}", &event).unwrap(),
        event["metadata"].to_string()
    );
    assert_eq!(
        render_prompt("{{json event}}", &event).unwrap(),
        event.to_string()
    );
}

#[test]
fn prompts_render_conditions_loops_and_inline_partials_as_literal_text() {
    let event = json!({
        "data": {"text": "<hello> & {{event.secret}}", "thread": "thread-1"},
        "attachments": [{"name": "first.txt"}, {"name": "second.txt"}]
    });
    let template = "{{#*inline \"attachment\"}}{{@index}}: {{name}}{{/inline}}\
        {{event.data.text}}\n\
        {{#if event.data.thread}}Thread: {{event.data.thread}}{{else}}No thread{{/if}}\n\
        {{#each event.attachments}}{{> attachment}};{{else}}No attachments{{/each}}";
    assert_eq!(
        render_prompt(template, &event).unwrap(),
        "<hello> & {{event.secret}}\nThread: thread-1\n0: first.txt;1: second.txt;"
    );
    assert_eq!(
        render_prompt(
            template,
            &json!({"data": {"text": "hello"}, "attachments": []})
        )
        .unwrap(),
        "hello\nNo thread\nNo attachments"
    );
    assert!(render_prompt("{{json event.missing}}", &event).is_err());
    let mut rule = definition("sample", "fn matches(event) { true }");
    rule.initial_prompt = "{{#if event.data.thread}}unclosed".into();
    assert!(validate(&rule).is_err());
}

#[test]
fn prompts_bound_output_empty_loop_work_and_recursive_partials() {
    assert_eq!(
        render_prompt(
            "{{event.data.text}}",
            &json!({"data": {"text": "x".repeat(65_536)}})
        )
        .unwrap()
        .len(),
        65_536
    );
    let error = render_prompt(
        "{{#each event.items}}{{this}}{{/each}}",
        &json!({"items": ["x".repeat(32_769), "x".repeat(32_769)]}),
    )
    .unwrap_err();
    assert!(error.contains("64 KiB"), "{error}");
    let error = render_prompt(
        "{{#each event.items}}{{#each @root.event.items}}{{/each}}{{/each}}done",
        &json!({"items": vec![0; 225]}),
    )
    .unwrap_err();
    assert!(error.contains("template evaluations"), "{error}");
    let error = render_prompt(
        "{{#*inline \"a\"}}{{> b}}{{/inline}}{{#*inline \"b\"}}{{> a}}{{/inline}}{{> a}}",
        &json!({}),
    )
    .unwrap_err();
    assert!(error.contains("rendering levels"), "{error}");
    assert!(render_prompt("{{event.data.text}}", &json!({"data": {"text": "\0"}})).is_err());
    assert!(render_prompt("{{#if event.missing}}hidden{{/if}}", &json!({})).is_err());
}

#[test]
fn sampled_predicates_keep_their_draw_across_engines_and_validate_probability_bounds() {
    let definition = definition(
        "sample",
        "fn matches(event) { sample(event.event_id, 0.8) }",
    );
    let mut accepted = 0;
    for index in 0..1000 {
        let event = json!({"event_id": format!("message:fixture:{index}")});
        let first = matches(&definition, &event).unwrap();
        assert_eq!(matches(&definition, &event).unwrap(), first);
        accepted += usize::from(first);
    }
    assert!((750..850).contains(&accepted), "{accepted}");
    assert!(!sample("event", 0.0).unwrap());
    assert!(sample("event", 1.0).unwrap());
    for probability in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
        assert!(sample("event", probability).is_err());
    }
    assert!(sample("", 0.8).is_err());
    assert!(sample(&"x".repeat(1025), 0.8).is_err());
}
