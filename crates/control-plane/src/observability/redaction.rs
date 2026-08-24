//! Redaction applied to every Sentry payload before it leaves the process.
//!
//! The rules mirror `apps/web/src/lib/observability/sentry.ts` so the browser
//! and the control plane hold the same line, plus the server-side secrets the
//! browser never sees: Piqae API keys, enrollment tokens, device credentials,
//! platform keys, and database connection URLs.
//!
//! Redaction is deliberately structural rather than opportunistic. Identity,
//! host attribution, request bodies, headers, cookies, stack-frame locals,
//! source context, and absolute build paths are dropped outright; whatever
//! survives is then pattern scrubbed. A vendor never receives a payload that
//! was not filtered here.

use anyhow::{Context as _, Result};
use regex::Regex;
use sentry::protocol::{Breadcrumb, Context, Event, Map, Request, Stacktrace, Value};
use std::borrow::Cow;
use url::Url;

/// Replacement for any value that must not leave the process.
const REDACTED: &str = "[redacted]";
/// Replacement for structures nested past [`MAX_DEPTH`].
const TRUNCATED: &str = "[truncated]";
/// Character budget for any single free-text field.
const MAX_TEXT_CHARS: usize = 1_000;
/// Maximum nesting walked in structured payloads.
const MAX_DEPTH: usize = 4;
/// Maximum array entries retained in structured payloads.
const MAX_ARRAY_ITEMS: usize = 50;
/// Maximum map entries retained in structured payloads.
const MAX_OBJECT_ENTRIES: usize = 100;
/// Base used only to normalize relative URLs; never emitted.
const URL_NORMALIZATION_BASE: &str = "https://piqae.invalid";

/// Key names whose value is never safe to report, regardless of content.
const SENSITIVE_KEY_PATTERN: &str = r"(?i)auth(?:orization)?|cookie|token|secret|password|passcode|api[-_]?key|apikey|session|credential|signature|dsn|nonce|salt|private[-_]?key|master[-_]?key|device[-_]?(?:code|key|id)|enrol(?:l)?ment|database[-_]?url|connection[-_]?string|document|content|payload|body|email|phone|address|username|full[-_]?name";

/// `key=value`, `key: value`, and `"key":"value"` pairs carrying a secret.
///
/// The value may carry an authorization scheme so that `Authorization: Bearer x`
/// is consumed whole instead of leaving the credential behind the scheme.
const SECRET_PAIR_PATTERN: &str = r#"(?i)(^|[?&"'\s,{\[])((?:access[-_]?token|refresh[-_]?token|id[-_]?token|bearer[-_]?token|token|secret|password|passcode|api[-_]?key|apikey|authorization|auth|cookie|dsn|private[-_]?key|master[-_]?key|webhook[-_]?key|device[-_]?(?:code|key)|enrol(?:l)?ment[-_]?token|database[-_]?url|connection[-_]?string)["']?\s*[:=]\s*["']?)((?:(?:bearer|basic|token)\s+)?[^&"',\s}\]]+)"#;

/// `Authorization`-style credentials embedded in free text.
const AUTH_VALUE_PATTERN: &str = r"(?i)\b(Bearer|Basic|Token)\s+[A-Za-z0-9._~+/=-]+";

/// Piqae-issued credentials: API keys, platform keys, device codes, and
/// enrollment tokens all share a `piq_`/`spl_` prefix (see `crates/auth`).
const PIQAE_CREDENTIAL_PATTERN: &str =
    r"(?i)\b(?:piq|spl)_(?:test|live|platform|dev|enr)_[A-Za-z0-9_-]+";

/// Database and broker connection URLs, credentials and all.
const CONNECTION_URL_PATTERN: &str = r"(?i)\b(postgresql|postgres|mysql|mariadb|mongodb\+srv|mongodb|rediss|redis|amqps|amqp|clickhouse|sqlserver|mssql)://\S*";

/// `scheme://user:password@host` credentials in any other URL.
const URL_CREDENTIALS_PATTERN: &str = r"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/@]+@";

const EMAIL_PATTERN: &str = r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b";

const UUID_PATTERN: &str =
    r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b";

/// Opaque path segments long enough to be an identifier or a secret.
const LONG_IDENTIFIER_PATTERN: &str = r"[A-Za-z0-9_-]{32,}";

/// Compiled scrubbing rules shared by the `before_send` and `before_breadcrumb`
/// hooks.
#[derive(Debug)]
pub struct Redactor {
    sensitive_key: Regex,
    secret_pair: Regex,
    auth_value: Regex,
    piqae_credential: Regex,
    connection_url: Regex,
    url_credentials: Regex,
    email: Regex,
    uuid: Regex,
    long_identifier: Regex,
}

impl Redactor {
    /// Compiles the scrubbing rules.
    ///
    /// Compilation is fallible rather than a lazily unwrapped static so that a
    /// malformed pattern surfaces at startup instead of panicking inside an
    /// error-reporting hook.
    pub fn new() -> Result<Self> {
        Ok(Self {
            sensitive_key: compile(SENSITIVE_KEY_PATTERN)?,
            secret_pair: compile(SECRET_PAIR_PATTERN)?,
            auth_value: compile(AUTH_VALUE_PATTERN)?,
            piqae_credential: compile(PIQAE_CREDENTIAL_PATTERN)?,
            connection_url: compile(CONNECTION_URL_PATTERN)?,
            url_credentials: compile(URL_CREDENTIALS_PATTERN)?,
            email: compile(EMAIL_PATTERN)?,
            uuid: compile(UUID_PATTERN)?,
            long_identifier: compile(LONG_IDENTIFIER_PATTERN)?,
        })
    }

    /// Scrubs free text and bounds its length.
    pub fn text(&self, value: &str) -> String {
        // Ordering matters. Named pairs are consumed first so that a later pass
        // cannot re-match the `[redacted]` marker a previous pass wrote and
        // corrupt the surrounding text.
        let scrubbed = self.secret_pair.replace_all(value, "${1}${2}[redacted]");
        let scrubbed = self
            .connection_url
            .replace_all(&scrubbed, "${1}://[redacted]");
        let scrubbed = self
            .url_credentials
            .replace_all(&scrubbed, "${1}[redacted]@");
        let scrubbed = self.email.replace_all(&scrubbed, REDACTED);
        let scrubbed = self.auth_value.replace_all(&scrubbed, "${1} [redacted]");
        let scrubbed = self.piqae_credential.replace_all(&scrubbed, REDACTED);
        truncate(scrubbed.into_owned())
    }

    /// Reduces a URL to scheme, host, and a de-identified path. Query strings
    /// and fragments are dropped rather than scrubbed.
    pub fn url(&self, value: &str) -> String {
        let absolute = Url::parse(value).ok();
        let parsed = absolute.clone().or_else(|| {
            Url::parse(URL_NORMALIZATION_BASE)
                .ok()
                .and_then(|base| base.join(value).ok())
        });
        let Some(parsed) = parsed else {
            let head = value.split(['?', '#']).next().unwrap_or_default();
            return self.text(head);
        };

        let path = self.uuid.replace_all(parsed.path(), ":id");
        let path = self.long_identifier.replace_all(&path, ":id");
        let path = self.email.replace_all(&path, REDACTED);

        if absolute.is_none() {
            return self.text(&path);
        }

        let mut origin = String::new();
        origin.push_str(parsed.scheme());
        origin.push_str("://");
        if let Some(host) = parsed.host_str() {
            origin.push_str(host);
        }
        if let Some(port) = parsed.port() {
            origin.push(':');
            origin.push_str(&port.to_string());
        }
        origin.push_str(&path);
        self.text(&origin)
    }

    /// Scrubs an event immediately before the transport serializes it.
    pub fn event(&self, mut event: Event<'static>) -> Event<'static> {
        // Identity and host attribution are dropped outright: this deployment
        // reports failures, not who or where.
        event.user = None;
        event.server_name = None;
        // Source context lines can echo rendered document text.
        event.template = None;

        event.message = event.message.as_deref().map(|value| self.text(value));
        event.culprit = event.culprit.as_deref().map(|value| self.text(value));
        event.transaction = event.transaction.as_deref().map(|value| self.text(value));

        if let Some(entry) = event.logentry.as_mut() {
            entry.message = self.text(&entry.message);
            entry.params = entry
                .params
                .iter()
                .take(MAX_ARRAY_ITEMS)
                .map(|param| self.value(param, "", 1))
                .collect();
        }

        event.fingerprint = Cow::Owned(
            event
                .fingerprint
                .iter()
                .map(|part| Cow::Owned(self.text(part)))
                .collect::<Vec<_>>(),
        );

        event.extra = self.json_map(&event.extra);
        event.tags = self.tag_map(&event.tags);
        event.contexts = self.contexts(std::mem::take(&mut event.contexts));
        event.request = event.request.take().map(|request| self.request(request));

        event.breadcrumbs.values = std::mem::take(&mut event.breadcrumbs.values)
            .into_iter()
            .filter_map(|breadcrumb| self.breadcrumb(breadcrumb))
            .collect();

        for exception in &mut event.exception.values {
            exception.value = exception.value.as_deref().map(|value| self.text(value));
            exception.module = exception.module.as_deref().map(|value| self.text(value));
            scrub_stacktrace(exception.stacktrace.as_mut());
            scrub_stacktrace(exception.raw_stacktrace.as_mut());
            if let Some(mechanism) = exception.mechanism.as_mut() {
                mechanism.description = mechanism
                    .description
                    .as_deref()
                    .map(|value| self.text(value));
                mechanism.data = self.json_map(&mechanism.data);
            }
        }

        for thread in &mut event.threads.values {
            scrub_stacktrace(thread.stacktrace.as_mut());
            scrub_stacktrace(thread.raw_stacktrace.as_mut());
        }
        scrub_stacktrace(event.stacktrace.as_mut());

        event
    }

    /// Scrubs a breadcrumb, or drops it when its category cannot carry
    /// anything useful without also carrying user input.
    pub fn breadcrumb(&self, mut breadcrumb: Breadcrumb) -> Option<Breadcrumb> {
        let category = breadcrumb.category.as_deref().unwrap_or_default();
        if category == "console" || category.starts_with("ui.") || breadcrumb.ty == "user" {
            return None;
        }
        breadcrumb.message = breadcrumb.message.as_deref().map(|value| self.text(value));
        breadcrumb.data = self.json_map(&breadcrumb.data);
        Some(breadcrumb)
    }

    fn request(&self, mut request: Request) -> Request {
        request.url = request
            .url
            .and_then(|url| Url::parse(&self.url(url.as_str())).ok());
        request.data = None;
        request.query_string = None;
        request.cookies = None;
        request.headers = Map::new();
        request.env = Map::new();
        request
    }

    fn contexts(&self, contexts: Map<String, Context>) -> Map<String, Context> {
        contexts
            .into_iter()
            .map(|(key, context)| {
                let context = match context {
                    Context::Other(other) => Context::Other(self.json_map(&other)),
                    Context::Device(mut device) => {
                        // `name` is the hostname on the contexts integration.
                        device.name = None;
                        Context::Device(device)
                    }
                    Context::App(mut app) => {
                        app.device_app_hash = None;
                        Context::App(app)
                    }
                    other => other,
                };
                (key, context)
            })
            .collect()
    }

    fn json_map(&self, map: &Map<String, Value>) -> Map<String, Value> {
        map.iter()
            .take(MAX_OBJECT_ENTRIES)
            .map(|(key, value)| (key.clone(), self.value(value, key, 1)))
            .collect()
    }

    fn tag_map(&self, tags: &Map<String, String>) -> Map<String, String> {
        tags.iter()
            .take(MAX_OBJECT_ENTRIES)
            .map(|(key, value)| (key.clone(), self.scalar(value, key)))
            .collect()
    }

    fn scalar(&self, value: &str, key: &str) -> String {
        if self.sensitive_key.is_match(key) {
            return REDACTED.to_owned();
        }
        if is_url_key(key) {
            return self.url(value);
        }
        self.text(value)
    }

    fn value(&self, value: &Value, key: &str, depth: usize) -> Value {
        if self.sensitive_key.is_match(key) {
            return Value::String(REDACTED.to_owned());
        }
        if depth > MAX_DEPTH {
            return Value::String(TRUNCATED.to_owned());
        }
        match value {
            Value::String(text) => Value::String(self.scalar(text, key)),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .take(MAX_ARRAY_ITEMS)
                    .map(|item| self.value(item, "", depth + 1))
                    .collect(),
            ),
            Value::Object(entries) => Value::Object(
                entries
                    .iter()
                    .take(MAX_OBJECT_ENTRIES)
                    .map(|(entry_key, entry_value)| {
                        (
                            entry_key.clone(),
                            self.value(entry_value, entry_key, depth + 1),
                        )
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

fn compile(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).with_context(|| format!("compile redaction pattern `{pattern}`"))
}

fn is_url_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("url") || key.contains("uri") || key == "from" || key == "to" || key == "path"
}

fn truncate(value: String) -> String {
    if value.chars().count() <= MAX_TEXT_CHARS {
        return value;
    }
    let mut truncated: String = value.chars().take(MAX_TEXT_CHARS).collect();
    truncated.push('…');
    truncated
}

/// Drops stack-frame locals, source context, registers, and absolute paths.
///
/// Locals and source lines could carry document bytes or key material. The
/// absolute path is a build-machine path: on a source build it embeds the
/// operator's home directory and account name, which the frame's `function`,
/// `package`, `filename`, and `lineno` already make unnecessary.
fn scrub_stacktrace(stacktrace: Option<&mut Stacktrace>) {
    let Some(stacktrace) = stacktrace else {
        return;
    };
    stacktrace.registers = Map::new();
    for frame in &mut stacktrace.frames {
        frame.vars = Map::new();
        frame.pre_context = Vec::new();
        frame.context_line = None;
        frame.post_context = Vec::new();
        frame.abs_path = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_TEXT_CHARS, Map, Redactor};
    use sentry::protocol::{Breadcrumb, Event, Exception, Frame, Request, Stacktrace, User};
    use serde_json::json;

    /// A representative sample of every credential shape the control plane
    /// handles. No test assertion may allow any of these to survive.
    const SECRETS: &[&str] = &[
        "piq_live_EXAMPLE_NOT_A_REAL_KEY",
        "piq_test_EXAMPLE_NOT_A_REAL_KEY",
        "piq_enr_Q2hhbGxlbmdlVG9rZW5NYXRlcmlhbA",
        "piq_dev_RGV2aWNlQ29kZVNlY3JldE1hdGVyaWFs",
        "piq_platform_EXAMPLE_NOT_A_REAL_KEY",
        "spl_live_9988776655443322110099887766554433",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl",
        "hunter2-the-database-password",
        "operator@example.com",
    ];

    fn redactor() -> Redactor {
        match Redactor::new() {
            Ok(redactor) => redactor,
            Err(error) => panic!("redaction rules must compile: {error}"),
        }
    }

    fn assert_clean(rendered: &str) {
        for secret in SECRETS {
            assert!(
                !rendered.contains(secret),
                "`{secret}` survived redaction in: {rendered}"
            );
        }
    }

    #[test]
    fn piqae_credentials_are_scrubbed_from_free_text() {
        let redactor = redactor();

        assert_eq!(
            redactor.text("rejected api key piq_live_EXAMPLE_NOT_A_REAL_KEY"),
            "rejected api key [redacted]"
        );
        assert_eq!(
            redactor.text("enrollment token piq_enr_Q2hhbGxlbmdlVG9rZW5NYXRlcmlhbA expired"),
            "enrollment token [redacted] expired"
        );
        assert_eq!(
            redactor.text("device code piq_dev_RGV2aWNlQ29kZVNlY3JldE1hdGVyaWFs not found"),
            "device code [redacted] not found"
        );
        assert_eq!(
            redactor.text("platform key spl_live_9988776655443322110099887766554433 revoked"),
            "platform key [redacted] revoked"
        );
    }

    #[test]
    fn authorization_material_is_scrubbed_but_the_scheme_survives() {
        let redactor = redactor();

        assert_eq!(
            redactor
                .text("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl"),
            "Authorization: [redacted]"
        );
        assert_eq!(
            redactor.text("header was Basic dXNlcjpwYXNzd29yZA=="),
            "header was Basic [redacted]"
        );
    }

    #[test]
    fn database_urls_never_survive() {
        let redactor = redactor();

        assert_eq!(
            redactor.text(
                "connect failed: postgres://piqae:hunter2-the-database-password@db.internal:5432/piqae"
            ),
            "connect failed: postgres://[redacted]"
        );
        assert_eq!(
            redactor.text("redis://default:hunter2-the-database-password@cache:6379/0"),
            "redis://[redacted]"
        );
        assert_eq!(
            redactor
                .text("fetch https://piqae:hunter2-the-database-password@objects.example.com/blob"),
            "fetch https://[redacted]@objects.example.com/blob"
        );
    }

    #[test]
    fn secret_key_value_pairs_are_scrubbed() {
        let redactor = redactor();

        assert_eq!(
            redactor.text("api_key=piq_live_EXAMPLE_NOT_A_REAL_KEY&limit=10"),
            "api_key=[redacted]&limit=10"
        );
        assert_eq!(
            redactor.text(r#"{"master_key":"c2VjcmV0","environment":"live"}"#),
            r#"{"master_key":"[redacted]","environment":"live"}"#
        );
        assert_eq!(
            redactor.text("password: hunter2-the-database-password"),
            "password: [redacted]"
        );
    }

    #[test]
    fn personal_data_is_scrubbed() {
        let redactor = redactor();

        assert_eq!(
            redactor.text("owner operator@example.com was denied"),
            "owner [redacted] was denied"
        );
    }

    #[test]
    fn ordinary_diagnostics_survive_intact() {
        let redactor = redactor();

        assert_eq!(
            redactor.text("print intent pi_01J0 rejected: printer offline"),
            "print intent pi_01J0 rejected: printer offline"
        );
        assert_eq!(
            redactor.text("spooler handoff accepted, delivery uncertain"),
            "spooler handoff accepted, delivery uncertain"
        );
    }

    #[test]
    fn free_text_is_bounded() {
        let redactor = redactor();
        let rendered = redactor.text(&"a".repeat(MAX_TEXT_CHARS * 3));

        assert_eq!(rendered.chars().count(), MAX_TEXT_CHARS + 1);
        assert!(rendered.ends_with('…'));
    }

    #[test]
    fn urls_lose_their_query_string_and_identifiers() {
        let redactor = redactor();

        assert_eq!(
            redactor.url(
                "https://api.piqae.example.com/v1/printjobs/018f3c2a-9f7b-4d31-9d6a-1f0f2a3b4c5d?api_key=piq_live_EXAMPLE_NOT_A_REAL_KEY"
            ),
            "https://api.piqae.example.com/v1/printjobs/:id"
        );
        assert_eq!(
            redactor.url("/v1/devices/piq_dev_RGV2aWNlQ29kZVNlY3JldE1hdGVyaWFs/claim"),
            "/v1/devices/:id/claim"
        );
        assert_eq!(
            redactor.url("/v1/users/operator@example.com"),
            "/v1/users/[redacted]"
        );
    }

    #[test]
    fn breadcrumbs_from_user_interaction_are_dropped() {
        let redactor = redactor();

        let console = Breadcrumb {
            category: Some("console".into()),
            ..Default::default()
        };
        assert!(redactor.breadcrumb(console).is_none());

        let click = Breadcrumb {
            category: Some("ui.click".into()),
            ..Default::default()
        };
        assert!(redactor.breadcrumb(click).is_none());

        let user = Breadcrumb {
            ty: "user".into(),
            ..Default::default()
        };
        assert!(redactor.breadcrumb(user).is_none());
    }

    #[test]
    fn breadcrumb_payloads_are_scrubbed() {
        let redactor = redactor();

        let mut breadcrumb = Breadcrumb {
            category: Some("http".into()),
            message: Some("retrying with piq_live_EXAMPLE_NOT_A_REAL_KEY".into()),
            ..Default::default()
        };
        breadcrumb.data.insert(
            "url".into(),
            json!("https://api.piqae.example.com/v1/printjobs?token=piq_test_EXAMPLE_NOT_A_REAL_KEY"),
        );
        breadcrumb.data.insert(
            "enrollment_token".into(),
            json!("piq_enr_Q2hhbGxlbmdlVG9rZW5NYXRlcmlhbA"),
        );

        let Some(scrubbed) = redactor.breadcrumb(breadcrumb) else {
            panic!("an http breadcrumb must survive");
        };

        assert_eq!(
            scrubbed.message.as_deref(),
            Some("retrying with [redacted]")
        );
        assert_eq!(
            scrubbed.data.get("url"),
            Some(&json!("https://api.piqae.example.com/v1/printjobs"))
        );
        assert_eq!(
            scrubbed.data.get("enrollment_token"),
            Some(&json!("[redacted]"))
        );
    }

    #[test]
    fn events_drop_identity_and_host_attribution() {
        let redactor = redactor();

        let event = Event {
            user: Some(User {
                id: Some("usr_01J0".into()),
                email: Some("operator@example.com".into()),
                ip_address: Some(sentry::protocol::IpAddress::Auto),
                ..Default::default()
            }),
            server_name: Some("piqae-prod-01.internal".into()),
            ..Default::default()
        };

        let scrubbed = redactor.event(event);

        assert!(scrubbed.user.is_none());
        assert!(scrubbed.server_name.is_none());
    }

    /// One event carrying every category of secret this control plane handles.
    fn dirty_event() -> Event<'static> {
        let frame = Frame {
            function: Some("piqae_control_plane::api::create_print_job".into()),
            filename: Some("crates/control-plane/src/api.rs".into()),
            vars: Map::from([(
                "api_key".to_owned(),
                json!("piq_live_EXAMPLE_NOT_A_REAL_KEY"),
            )]),
            abs_path: Some("/Users/operator/piqae/crates/control-plane/src/api.rs".into()),
            context_line: Some("let document = b\"%PDF-1.7 confidential\";".into()),
            pre_context: vec!["let key = piq_test_EXAMPLE_NOT_A_REAL_KEY;".into()],
            ..Default::default()
        };

        let exception = Exception {
            ty: "RepositoryError".into(),
            value: Some(
                "insert failed for postgres://piqae:hunter2-the-database-password@db:5432/piqae"
                    .into(),
            ),
            stacktrace: Some(Stacktrace {
                frames: vec![frame],
                ..Default::default()
            }),
            ..Default::default()
        };

        let request = Request {
            method: Some("POST".into()),
            url: "https://api.piqae.example.com/v1/printjobs/018f3c2a-9f7b-4d31-9d6a-1f0f2a3b4c5d?token=piq_test_EXAMPLE_NOT_A_REAL_KEY"
                .parse()
                .ok(),
            cookies: Some("session=piq_test_EXAMPLE_NOT_A_REAL_KEY".into()),
            data: Some("%PDF-1.7 confidential document bytes".into()),
            query_string: Some("token=piq_test_EXAMPLE_NOT_A_REAL_KEY".into()),
            headers: Map::from([(
                "authorization".to_owned(),
                "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl".to_owned(),
            )]),
            env: Map::from([(
                "DATABASE_URL".to_owned(),
                "postgres://piqae:hunter2-the-database-password@db:5432/piqae".to_owned(),
            )]),
        };

        Event {
            message: Some(
                "rejected piq_live_EXAMPLE_NOT_A_REAL_KEY for operator@example.com"
                    .into(),
            ),
            transaction: Some("POST /v1/printjobs".into()),
            server_name: Some("piqae-prod-01.internal".into()),
            exception: vec![exception].into(),
            request: Some(request),
            tags: Map::from([
                (
                    "device_key".to_owned(),
                    "piq_platform_EXAMPLE_NOT_A_REAL_KEY".to_owned(),
                ),
                ("route".to_owned(), "/v1/printjobs".to_owned()),
            ]),
            extra: Map::from([
                (
                    "document_content".to_owned(),
                    json!("%PDF-1.7 confidential document bytes"),
                ),
                (
                    "database_url".to_owned(),
                    json!("postgres://piqae:hunter2-the-database-password@db:5432/piqae"),
                ),
                (
                    "context".to_owned(),
                    json!({
                        "enrollment": "piq_enr_Q2hhbGxlbmdlVG9rZW5NYXRlcmlhbA",
                        "attempts": 3,
                        "note": "retry after Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl"
                    }),
                ),
            ]),
            ..Default::default()
        }
    }

    #[test]
    fn events_scrub_messages_tags_and_extra() {
        let scrubbed = redactor().event(dirty_event());

        assert_eq!(
            scrubbed.message.as_deref(),
            Some("rejected [redacted] for [redacted]")
        );
        assert_eq!(
            scrubbed.tags.get("device_key").map(String::as_str),
            Some("[redacted]")
        );
        assert_eq!(
            scrubbed.tags.get("route").map(String::as_str),
            Some("/v1/printjobs")
        );
        assert_eq!(
            scrubbed.extra.get("document_content"),
            Some(&json!("[redacted]"))
        );
        assert_eq!(
            scrubbed.extra.get("database_url"),
            Some(&json!("[redacted]"))
        );
        assert_eq!(
            scrubbed.extra.get("context"),
            Some(&json!({
                "enrollment": "[redacted]",
                "attempts": 3,
                "note": "retry after Bearer [redacted]"
            }))
        );
    }

    #[test]
    fn events_reduce_requests_to_a_method_and_a_de_identified_path() {
        let scrubbed = redactor().event(dirty_event());

        let Some(request) = scrubbed.request.as_ref() else {
            panic!("the request stub must survive");
        };
        assert_eq!(request.method.as_deref(), Some("POST"));
        assert_eq!(
            request.url.as_ref().map(url::Url::as_str),
            Some("https://api.piqae.example.com/v1/printjobs/:id")
        );
        assert!(request.cookies.is_none());
        assert!(request.data.is_none());
        assert!(request.query_string.is_none());
        assert!(request.headers.is_empty());
        assert!(request.env.is_empty());
    }

    #[test]
    fn events_keep_stack_frames_but_drop_locals_and_source() {
        let scrubbed = redactor().event(dirty_event());

        let Some(exception) = scrubbed.exception.values.first() else {
            panic!("the exception must survive");
        };
        assert_eq!(
            exception.value.as_deref(),
            Some("insert failed for postgres://[redacted]")
        );

        let Some(frame) = exception
            .stacktrace
            .as_ref()
            .and_then(|stacktrace| stacktrace.frames.first())
        else {
            panic!("the frame must survive");
        };
        assert_eq!(
            frame.function.as_deref(),
            Some("piqae_control_plane::api::create_print_job")
        );
        assert_eq!(
            frame.filename.as_deref(),
            Some("crates/control-plane/src/api.rs")
        );
        assert!(frame.vars.is_empty());
        assert!(frame.context_line.is_none());
        assert!(frame.pre_context.is_empty());
        assert!(
            frame.abs_path.is_none(),
            "the build machine's absolute path survived"
        );
    }

    #[test]
    fn the_serialized_payload_carries_no_secret() {
        let scrubbed = redactor().event(dirty_event());

        // The serialized envelope is what actually leaves the process.
        let Ok(rendered) = serde_json::to_string(&scrubbed) else {
            panic!("a scrubbed event must serialize");
        };

        assert_clean(&rendered);
        assert!(
            !rendered.contains("%PDF"),
            "document bytes survived: {rendered}"
        );
        assert!(
            !rendered.contains("piqae-prod-01"),
            "host attribution survived: {rendered}"
        );
        assert!(
            !rendered.contains("/Users/operator"),
            "a build machine path survived: {rendered}"
        );
    }

    #[test]
    fn structured_payloads_are_bounded_in_depth_and_width() {
        let redactor = redactor();

        let mut event = Event::default();
        event.extra.insert(
            "deep".into(),
            json!({"a":{"b":{"c":{"d":{"e":"too far"}}}}}),
        );
        event.extra.insert(
            "wide".into(),
            json!((0..200).map(serde_json::Value::from).collect::<Vec<_>>()),
        );

        let scrubbed = redactor.event(event);

        assert_eq!(
            scrubbed.extra.get("deep"),
            Some(&json!({"a":{"b":{"c":{"d":"[truncated]"}}}}))
        );
        let Some(serde_json::Value::Array(wide)) = scrubbed.extra.get("wide") else {
            panic!("the array must survive");
        };
        assert_eq!(wide.len(), super::MAX_ARRAY_ITEMS);
    }
}
