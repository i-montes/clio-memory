#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode

use std::path::Path;
use std::sync::{Arc, Mutex};

use clio_memory::Relation;
use clio_memory::temis::*;
use serde_json::json;

type Calls = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// A fake token, built at run time so no token-shaped literal sits in the repository.
fn token() -> String {
    format!("{}-{}", "tk", "0123456789abcdef0123456789")
}

fn cfg() -> Config {
    Config {
        url: "https://h/temis".into(),
        token: token(),
        model: "temis-latest".into(),
    }
}

/// Answers each call with the next (code, body) (the last one repeats) and records (path, body).
fn fake(replies: Vec<(u16, &str)>) -> (Box<Post>, Calls) {
    let calls: Calls = Arc::default();
    let seen = calls.clone();
    let replies: Vec<(u16, String)> = replies
        .into_iter()
        .map(|(c, b)| (c, b.to_owned()))
        .collect();
    let post: Box<Post> = Box::new(move |_, path, body| {
        let mut seen = seen.lock().unwrap();
        let n = seen.len().min(replies.len() - 1);
        seen.push((path.to_owned(), body.map(str::to_owned)));
        Ok(replies[n].clone())
    });
    (post, calls)
}

fn noul(v: &str) -> String {
    format!(r#"{{"answers":{{"se_conserva":{{"type":"noul","noul":{v}}}}}}}"#)
}

fn choice(c: &str) -> String {
    format!(r#"{{"answers":{{"relacion":{{"type":"choice","choice":"{c}","confidence":0.97}}}}}}"#)
}

#[test]
fn the_question_is_the_one_temis_was_trained_on() {
    let (post, calls) = fake(vec![(
        200,
        r#"{"model":"temis-2.4.0","answers":{"se_conserva":{"type":"noul","noul":0.79}},"usage":{"input_tokens":1,"output_tokens":0}}"#,
    )]);
    let t = Temis::new(cfg(), post);
    assert_eq!(t.preserved("uno", "uno y dos"), Some(0.79));
    let (path, body) = calls.lock().unwrap()[0].clone();
    let body: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
    assert_eq!(path, "/v1/decide");
    assert_eq!(body["model"], "temis-latest");
    assert_eq!(
        body["state"],
        json!({"original": "uno", "unido": "uno y dos"})
    );
    assert_eq!(
        body["questions"]["se_conserva"]["instructions"],
        "Todo lo que dice `original` está también en `unido`, aunque sea con otras palabras."
    );
}

#[test]
fn the_relation_question_carries_its_five_criteria() {
    let (post, calls) = fake(vec![(200, &choice("related"))]);
    Temis::new(cfg(), post).relation("p", "o");
    let body = calls.lock().unwrap()[0].1.clone().unwrap();
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["state"], json!({"propuesta": "p", "otra": "o"}));
    let criteria = body["questions"]["relacion"]["criteria"]
        .as_object()
        .unwrap();
    let mut keys: Vec<_> = criteria.keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "compatible",
            "conflicts_with",
            "ninguna",
            "related",
            "scoped"
        ]
    );
}

#[test]
fn the_token_goes_to_curl_on_stdin_escaped_and_never_as_an_argument() {
    let c = Config {
        url: "https://h/temis".into(),
        token: "tok\"en\\x-0123456789abcdef0123".into(),
        model: "temis-latest".into(),
    };
    let text = curl_config(&c, "/v1/decide", Some(r#"{"a":"b \"c\""}"#));
    assert!(text.contains(r#"url = "https://h/temis/v1/decide""#));
    assert!(text.contains(r#"header = "Authorization: Bearer tok\"en\\x-0123456789abcdef0123""#));
    assert!(text.contains(r#"data-binary = "{\"a\":\"b \\\"c\\\"\"}""#));
    assert!(text.contains("max-time = 10"));
    // A line break in a value cannot start a new line of the config.
    let c = Config {
        token: "a\nurl = \"x\"".into(),
        ..c
    };
    assert_eq!(
        curl_config(&c, "/v1/model", None)
            .matches("\nurl = ")
            .count(),
        0
    );
}

#[test]
fn a_choice_maps_to_a_relation() {
    for (said, want) in [
        ("conflicts_with", Relation::ConflictsWith),
        ("ninguna", Relation::NotConflict),
        ("scoped", Relation::Scoped),
        ("compatible", Relation::Compatible),
        ("related", Relation::Related),
    ] {
        let (post, _) = fake(vec![(200, &choice(said))]);
        let got = Temis::new(cfg(), post).relation("a", "b");
        assert_eq!(got, Some((want, 0.97)), "{said}");
    }
}

#[test]
fn an_odd_answer_is_no_answer() {
    let relation = |c: &str| format!(r#"{{"answers":{{"relacion":{c}}}}}"#);
    let odd_noul = [
        r#"{"model":"x"}"#.to_owned(),
        r#"{"answers":[]}"#.into(),
        r#"{"answers":{}}"#.into(),
        noul("1.4"),
        noul("-0.1"),
        noul(r#""sí""#),
        noul("null"),
        r#"{"answers":{"se_conserva":{"type":"choice","noul":0.9}}}"#.into(),
        "{no es json".into(),
        String::new(),
    ];
    for body in &odd_noul {
        let (post, _) = fake(vec![(200, body)]);
        assert_eq!(Temis::new(cfg(), post).preserved("a", "b"), None, "{body}");
    }
    let odd_choice = [
        relation(r#"{"type":"choice","choice":"otra_cosa","confidence":0.9}"#),
        relation(r#"{"type":"choice","choice":"related","confidence":1.5}"#),
        relation(r#"{"type":"choice","choice":"related"}"#),
        relation(r#"{"type":"choice","choice":7,"confidence":0.9}"#),
        relation(r#"{"type":"noul","choice":"related","confidence":0.9}"#),
        noul("0.9"),
        "[1]".into(),
    ];
    for body in &odd_choice {
        let (post, _) = fake(vec![(200, body)]);
        assert_eq!(Temis::new(cfg(), post).relation("a", "b"), None, "{body}");
    }
}

#[test]
fn an_odd_answer_counts_toward_the_switch() {
    let (post, calls) = fake(vec![(200, r#"{"answers":{}}"#)]);
    let t = Temis::new(cfg(), post);
    for _ in 0..4 {
        assert_eq!(t.preserved("a", "b"), None);
    }
    assert_eq!(calls.lock().unwrap().len(), 3);
}

#[test]
fn an_error_names_its_kind_and_never_the_token() {
    let body = format!(
        r#"{{"detail":{{"error_type":"authentication_error","message":"bad token {}"}}}}"#,
        token()
    );
    let (post, _) = fake(vec![(401, &body)]);
    let err = Temis::new(cfg(), post)
        .decide(&json!({}), &json!({}))
        .unwrap_err();
    assert!(
        err.contains("authentication_error") && err.contains("401"),
        "{err}"
    );
    assert!(
        !err.contains(&token()) && !err.contains("bad token"),
        "{err}"
    );
    // A kind that is not a plain word is not repeated.
    let body = format!(r#"{{"detail":{{"error_type":"{}"}}}}"#, token());
    let (post, _) = fake(vec![(500, &body)]);
    let err = Temis::new(cfg(), post)
        .decide(&json!({}), &json!({}))
        .unwrap_err();
    assert!(!err.contains(&token()), "{err}");
}

#[test]
fn three_failures_in_a_row_turn_it_off() {
    let (post, calls) = fake(vec![(
        503,
        r#"{"detail":{"error_type":"overloaded_error"}}"#,
    )]);
    let t = Temis::new(cfg(), post);
    for _ in 0..4 {
        assert_eq!(t.preserved("a", "b"), None);
    }
    assert_eq!(calls.lock().unwrap().len(), 3);
    assert!(t.decide(&json!({}), &json!({})).is_err());
    assert_eq!(calls.lock().unwrap().len(), 3);
}

#[test]
fn a_success_resets_the_count() {
    let ok = noul("0.9");
    let (post, calls) = fake(vec![
        (503, "{}"),
        (503, "{}"),
        (200, &ok),
        (503, "{}"),
        (503, "{}"),
        (200, &ok),
    ]);
    let t = Temis::new(cfg(), post);
    let got: Vec<_> = (0..6).map(|_| t.preserved("a", "b")).collect();
    assert_eq!(got, [None, None, Some(0.9), None, None, Some(0.9)]);
    assert_eq!(calls.lock().unwrap().len(), 6);
}

#[test]
fn long_texts_are_cut_at_4000_characters() {
    let (post, calls) = fake(vec![(200, &noul("0.5"))]);
    let long = "á".repeat(5000);
    Temis::new(cfg(), post).preserved(&long, "b");
    let body = calls.lock().unwrap()[0].1.clone().unwrap();
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        body["state"]["original"].as_str().unwrap().chars().count(),
        4000
    );
}

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| {
        pairs
            .iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| (*v).to_owned())
    }
}

#[test]
fn the_file_is_private_and_the_environment_wins() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    assert!(load(&data, &env(&[])).is_none());
    assert!(!remove(&data).unwrap());

    save(&data, &cfg()).unwrap();
    let mode = std::fs::metadata(config_path(&data))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    // Saving again over an old, looser file leaves it private.
    std::fs::set_permissions(config_path(&data), std::fs::Permissions::from_mode(0o644)).unwrap();
    save(&data, &cfg()).unwrap();
    let mode = std::fs::metadata(config_path(&data))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);

    let from_file = load(&data, &env(&[])).unwrap();
    assert_eq!(
        (from_file.url.as_str(), from_file.token.as_str()),
        ("https://h/temis", token().as_str())
    );
    let both = [
        ("CLIO_MEMORY_TEMIS_URL", "https://v/t"),
        ("CLIO_MEMORY_TEMIS_TOKEN", "from-env"),
    ];
    let from_env = load(&data, &env(&both)).unwrap();
    assert_eq!(
        (from_env.url.as_str(), from_env.token.as_str()),
        ("https://v/t", "from-env")
    );
    let one = load(&data, &env(&both[..1])).unwrap();
    assert_eq!(one.url, "https://h/temis");
    // A URL from the environment is checked like setup checks one: `http` to another machine is
    // as if the variables were not set (the file's own was checked when it was saved).
    let clear = [
        ("CLIO_MEMORY_TEMIS_URL", "http://api.x"),
        ("CLIO_MEMORY_TEMIS_TOKEN", "from-env"),
    ];
    let fallback = load(&data, &env(&clear)).unwrap();
    assert_eq!(
        (fallback.url.as_str(), fallback.token.as_str()),
        ("https://h/temis", token().as_str())
    );
    assert!(load(&dir.path().join("none"), &env(&clear)).is_none());
    let slashed = [
        ("CLIO_MEMORY_TEMIS_URL", "https://v/t/"),
        ("CLIO_MEMORY_TEMIS_TOKEN", "from-env"),
    ];
    assert_eq!(load(&data, &env(&slashed)).unwrap().url, "https://v/t");
    // Without a file, the variables alone work; one alone is nothing.
    let empty = dir.path().join("none");
    assert_eq!(load(&empty, &env(&both)).unwrap().model, "temis-latest");
    assert!(load(&empty, &env(&both[..1])).is_none());

    assert!(remove(&data).unwrap());
    assert!(!config_path(&data).exists());
    assert_leftovers(&data);
}

fn assert_leftovers(data: &Path) {
    let names: Vec<_> = std::fs::read_dir(data).unwrap().flatten().collect();
    assert!(names.is_empty(), "{names:?}");
}

#[test]
fn only_https_or_this_machine() {
    assert_eq!(
        check_url("https://api.x/temis/", false),
        Ok("https://api.x/temis".into())
    );
    assert_eq!(
        check_url("http://localhost:8765", false),
        Ok("http://localhost:8765".into())
    );
    assert!(check_url("http://127.0.0.1:1", false).is_ok());
    assert!(check_url("http://[::1]:9", false).is_ok());
    assert!(check_url("http://api.x", false).is_err());
    assert_eq!(check_url("http://api.x", true), Ok("http://api.x".into()));
    for bad in [
        "ftp://x",
        "api.x",
        "https://",
        "http://localhost@evil.com",
        "http://localhost.evil.com",
        "https://a b",
        "https://a\"b",
        "https://api.x/t?x=1",
        "https://api.x:port",
    ] {
        assert!(check_url(bad, false).is_err(), "{bad}");
    }
}

#[test]
fn debug_never_prints_the_token() {
    assert!(!format!("{:?}", cfg()).contains(&token()));
}

#[test]
fn curl_runs_with_the_config_on_stdin() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (input, args) = (dir.path().join("stdin"), dir.path().join("args"));
    let curl = dir.path().join("curl");
    std::fs::write(
        &curl,
        format!(
            "#!/bin/sh\ncat > '{}'\nfor a in \"$@\"; do echo \"$a\"; done > '{}'\nprintf '{{\"status\":\"ok\",\"model\":\"t\"}}\\n200'\n",
            input.display(),
            args.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();

    let t = Temis::new(cfg(), curl_post(curl));
    assert_eq!(t.health(), Ok("t".into()));
    assert_eq!(std::fs::read_to_string(&args).unwrap(), "--config\n-\n");
    let stdin = std::fs::read_to_string(&input).unwrap();
    assert!(
        stdin.contains(r#"url = "https://h/temis/health""#),
        "{stdin}"
    );
    assert!(!stdin.contains("Authorization"), "{stdin}");
    // With the token: it is on stdin, and still not in the arguments.
    assert_eq!(t.model(), Ok("t".into()));
    assert!(std::fs::read_to_string(&input).unwrap().contains(&token()));
    assert!(!std::fs::read_to_string(&args).unwrap().contains(&token()));
}

#[test]
fn a_curl_that_fails_says_so_without_the_token() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let curl = dir.path().join("curl");
    std::fs::write(
        &curl,
        format!(
            "#!/bin/sh\ncat > /dev/null\necho 'curl: (6) could not resolve {}' >&2\nexit 6\n",
            token()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = Temis::new(cfg(), curl_post(curl)).health().unwrap_err();
    assert!(err.contains("could not resolve"), "{err}");
    assert!(!err.contains(&token()), "{err}");

    let missing = dir.path().join("no-curl");
    let err = Temis::new(cfg(), curl_post(missing)).health().unwrap_err();
    assert!(!err.contains(&token()), "{err}");
}
