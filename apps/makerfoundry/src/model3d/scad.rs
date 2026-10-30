//! Parametric models (issue #35): a creator's OpenSCAD file whose parameters visitors set before
//! downloading an STL made to them. The parameters are read here, by our own reader of the
//! Customizer's syntax as OpenSCAD's manual describes it
//! (<https://en.wikibooks.org/wiki/OpenSCAD_User_Manual/Customizer>): `name = literal;` lines
//! before the first `{`, a `// description` line above, a trailing `// [..]` comment giving a
//! slider (`[max]`, `[min:max]`, `[min:step:max]`) or a dropdown (`[a, b]`, `[10:Small, 20:Big]`),
//! `/* [Tab] */` groups, and `/* [Hidden] */` ones left out.
//!
//! The STL is made by the owner's render service, a separate, sandboxed program running OpenSCAD
//! (GPL: never linked into ours, never copied), with its own time, memory and output caps and no
//! network: `POST <MF_SCAD_SERVICE>/render` with JSON `{"scad": "<the file>", "parameters":
//! {"name": value}}` (only parameters the file declares, each checked against its declaration
//! here first), `Authorization: Bearer <MF_SCAD_SERVICE_KEY>` when a key is set → `200` with a
//! binary STL, read again by our own STL reader with its limits before anyone gets it. Off until
//! `MF_SCAD_SERVICE` is set.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};

use super::mesh::MAX_FILE_BYTES;
use crate::AppState;
use crate::accounts::{current_user, forbidden, same_origin};
use crate::pages::escape;

/// The most parameters read from one file.
pub const MAX_PARAMETERS: usize = 100;
/// The longest text a text parameter takes.
const MAX_TEXT: usize = 200;
/// The most choices a dropdown offers.
const MAX_CHOICES: usize = 100;

/// What kind of value a parameter takes.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A number, within `min..=max` when they're given.
    Number {
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
    },
    /// One of these values (as written), each with its label.
    Choice(Vec<(Value, String)>),
    Bool,
    /// Text of at most this many characters.
    Text(usize),
    /// 2 to 4 numbers.
    Vector(usize),
}

/// A parameter's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Bool(bool),
    Text(String),
    Vector(Vec<f64>),
}

impl Value {
    /// As JSON for the render service.
    pub fn json(&self) -> serde_json::Value {
        match self {
            Value::Number(n) => serde_json::json!(n),
            Value::Bool(b) => serde_json::json!(b),
            Value::Text(t) => serde_json::json!(t),
            Value::Vector(v) => serde_json::json!(v),
        }
    }

    /// As a form field shows it.
    pub fn shown(&self) -> String {
        match self {
            Value::Number(n) => number(*n),
            Value::Bool(b) => b.to_string(),
            Value::Text(t) => t.clone(),
            Value::Vector(v) => v.iter().map(|n| number(*n)).collect::<Vec<_>>().join(", "),
        }
    }
}

fn number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

/// One parameter a file declares.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub description: String,
    pub group: String,
    pub kind: Kind,
    pub default: Value,
}

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && s.len() <= 64
        && !s.starts_with('$')
}

fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() || s.len() > 40 || !s.chars().all(|c| c.is_ascii_digit() || "+-.eE".contains(c)) {
        return None;
    }
    s.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// A literal value: a number, `true`/`false`, a `"string"` or a vector of 2 to 4 numbers.
fn literal(s: &str) -> Option<Value> {
    let s = s.trim();
    if s == "true" || s == "false" {
        return Some(Value::Bool(s == "true"));
    }
    if let Some(inner) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        if inner.contains('"') || inner.contains('\\') || inner.chars().count() > MAX_TEXT {
            return None;
        }
        return Some(Value::Text(inner.to_string()));
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        let v: Option<Vec<f64>> = inner.split(',').map(parse_number).collect();
        return v.filter(|v| (2..=4).contains(&v.len())).map(Value::Vector);
    }
    parse_number(s).map(Value::Number)
}

/// A trailing `[..]` annotation, for a default value's kind.
fn annotate(default: &Value, note: Option<&str>) -> Option<Kind> {
    let inner = note
        .and_then(|n| n.trim().strip_prefix('['))
        .and_then(|n| n.strip_suffix(']'));
    match default {
        Value::Bool(_) => Some(Kind::Bool),
        Value::Vector(v) => Some(Kind::Vector(v.len())),
        Value::Text(_) => match inner {
            Some(list) => choices(list, false),
            None => Some(Kind::Text(MAX_TEXT)),
        },
        Value::Number(_) => {
            let Some(inner) = inner else {
                return Some(Kind::Number {
                    min: None,
                    max: None,
                    step: None,
                });
            };
            if inner.contains(',') || inner.split(':').any(|p| parse_number(p).is_none()) {
                return choices(inner, true);
            }
            let parts: Vec<f64> = inner.split(':').filter_map(parse_number).collect();
            let (min, step, max) = match parts.as_slice() {
                [max] => (Some(0.0), None, Some(*max)),
                [min, max] => (Some(*min), None, Some(*max)),
                [min, step, max] => (Some(*min), Some(*step).filter(|s| *s > 0.0), Some(*max)),
                _ => return None,
            };
            if min.zip(max).is_some_and(|(a, b)| a > b) {
                return None;
            }
            Some(Kind::Number { min, max, step })
        }
    }
}

/// A dropdown: `a, b, c` or `value:Label, …`.
fn choices(list: &str, numbers: bool) -> Option<Kind> {
    let mut out = Vec::new();
    for item in list.split(',').take(MAX_CHOICES + 1) {
        let (value, label) = match item.split_once(':') {
            Some((v, l)) => (v.trim(), l.trim()),
            None => (item.trim(), item.trim()),
        };
        if value.is_empty() || label.chars().count() > 80 {
            return None;
        }
        let value = if numbers {
            Value::Number(parse_number(value)?)
        } else {
            if value.contains('"') || value.contains('\\') || value.chars().count() > MAX_TEXT {
                return None;
            }
            Value::Text(value.to_string())
        };
        out.push((value, label.to_string()));
    }
    (!out.is_empty() && out.len() <= MAX_CHOICES).then_some(Kind::Choice(out))
}

/// The parameters an OpenSCAD file declares for the Customizer.
pub fn parameters(source: &str) -> Vec<Parameter> {
    let mut out: Vec<Parameter> = Vec::new();
    let mut group = "Parameters".to_string();
    let mut description = String::new();
    for line in source.lines().take(5000) {
        let trimmed = line.trim();
        if let Some(name) = trimmed
            .strip_prefix("/*")
            .and_then(|r| r.strip_suffix("*/"))
            .map(str::trim)
            .and_then(|r| r.strip_prefix('['))
            .and_then(|r| r.strip_suffix(']'))
        {
            group = name.trim().chars().take(60).collect();
            description.clear();
            continue;
        }
        // the Customizer stops at the first `{`
        if line.split("//").next().is_some_and(|code| code.contains('{')) {
            break;
        }
        if let Some(text) = line.strip_prefix("//") {
            description = text.trim().chars().take(200).collect();
            continue;
        }
        let (code, note) = match line.split_once("//") {
            Some((c, n)) => (c, Some(n)),
            None => (line, None),
        };
        let Some((name, value)) = code.trim().strip_suffix(';').and_then(|c| c.split_once('=')) else {
            description.clear();
            continue;
        };
        let name = name.trim();
        let taken = group.eq_ignore_ascii_case("hidden") || !is_name(name) || out.iter().any(|p| p.name == name);
        if let (false, Some(default)) = (taken, literal(value))
            && let Some(kind) = annotate(&default, note)
        {
            out.push(Parameter {
                name: name.to_string(),
                description: std::mem::take(&mut description),
                group: group.clone(),
                kind,
                default,
            });
            if out.len() >= MAX_PARAMETERS {
                break;
            }
        }
        description.clear();
    }
    out
}

/// The values a form gives, each checked against its parameter; a parameter the form leaves out
/// keeps its default. Anything else is refused.
pub fn values(params: &[Parameter], form: &HashMap<String, String>) -> Result<Vec<(String, Value)>, String> {
    let mut out = Vec::new();
    for p in params {
        let given = form.get(&format!("p_{}", p.name)).map(|s| s.trim());
        let value = match (&p.kind, given) {
            (Kind::Bool, given) => Value::Bool(given == Some("true")),
            (_, None) => p.default.clone(),
            (Kind::Number { min, max, step: _ }, Some(s)) => {
                let n = parse_number(s).ok_or_else(|| format!("{} must be a number", p.name))?;
                if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                    return Err(format!("{} is out of its range", p.name));
                }
                Value::Number(n)
            }
            (Kind::Choice(list), Some(s)) => list
                .iter()
                .map(|(v, _)| v)
                .find(|v| v.shown() == s)
                .cloned()
                .ok_or_else(|| format!("{} isn't one of its choices", p.name))?,
            (Kind::Text(max), Some(s)) => {
                if s.chars().count() > *max || s.chars().any(|c| c.is_control() || c == '"' || c == '\\') {
                    return Err(format!("{} must be plain text of at most {max} characters", p.name));
                }
                Value::Text(s.to_string())
            }
            (Kind::Vector(n), Some(s)) => {
                let v: Option<Vec<f64>> = s.split(',').map(parse_number).collect();
                Value::Vector(
                    v.filter(|v| v.len() == *n)
                        .ok_or_else(|| format!("{} must be {n} numbers", p.name))?,
                )
            }
        };
        out.push((p.name.clone(), value));
    }
    Ok(out)
}

/// The render service.
#[derive(Clone)]
pub struct Renderer {
    url: String,
    key: Option<String>,
    http: reqwest::Client,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the key)
        f.write_str("Renderer")
    }
}

impl Renderer {
    pub fn new(url: &str, key: Option<&str>) -> Option<Self> {
        let url = url.trim().trim_end_matches('/');
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(Renderer {
            url: url.to_string(),
            key: key.map(str::trim).filter(|k| !k.is_empty()).map(str::to_string),
            http,
        })
    }

    /// An STL of `scad` with these values, at most `MAX_FILE_BYTES` (not yet read: the caller
    /// reads it with `stl::read`).
    pub async fn render(&self, scad: &str, values: &[(String, Value)]) -> Result<Vec<u8>, String> {
        let parameters: serde_json::Map<String, serde_json::Value> =
            values.iter().map(|(n, v)| (n.clone(), v.json())).collect();
        let mut request = self
            .http
            .post(format!("{}/render", self.url))
            .header(reqwest::header::ACCEPT, "model/stl")
            .json(&serde_json::json!({ "scad": scad, "parameters": parameters }));
        if let Some(key) = &self.key {
            request = request.bearer_auth(key);
        }
        let mut answer = request
            .send()
            .await
            .map_err(|e| format!("render: {}", e.without_url()))?;
        if !answer.status().is_success() {
            return Err(format!("render: {}", answer.status()));
        }
        if answer.content_length().is_some_and(|n| n > MAX_FILE_BYTES as u64) {
            return Err("render: the STL is too large".into());
        }
        let mut stl = Vec::new();
        while let Some(chunk) = answer
            .chunk()
            .await
            .map_err(|e| format!("render: {}", e.without_url()))?
        {
            if stl.len() + chunk.len() > MAX_FILE_BYTES {
                return Err("render: the STL is too large".into());
            }
            stl.extend_from_slice(&chunk);
        }
        Ok(stl)
    }
}

// ---------------------------------------------------------------- the pages

/// The "Customize" links for a model's OpenSCAD files, when the render service is set and the
/// viewer may download the model.
pub fn links_html(state: &AppState, id: i64, files: &[super::upload::ModelFile], may_download: bool) -> String {
    if state.scad.is_none() || !may_download {
        return String::new();
    }
    let links: String = files
        .iter()
        .filter(|f| f.format == "scad")
        .map(|f| {
            format!(
                r#"<li><a href="/d/{id}/customize/{fid}">Customize {name}</a></li>"#,
                fid = f.id,
                name = escape(&f.file_name)
            )
        })
        .collect();
    if links.is_empty() {
        String::new()
    } else {
        format!(
            r#"<h2>Make it your size</h2><p>Set this model's parameters and download an STL made to them.</p><ul class="customize">{links}</ul>"#
        )
    }
}

/// The model, its `.scad` file's name and text, if the viewer may customize it.
async fn source(
    state: &AppState,
    headers: &HeaderMap,
    id: i64,
    file: i64,
) -> Result<(crate::designs::DesignRow, String, String), Response> {
    let not_found = || crate::designs::not_found();
    let viewer = current_user(state, headers).await;
    let (Some(_), Some(pool), Some(storage)) = (state.scad.as_ref(), state.db.as_ref(), state.storage.as_ref()) else {
        return Err(not_found());
    };
    let Ok(Some(d)) = crate::designs::design(pool, id).await else {
        return Err(not_found());
    };
    let (seen, may) = crate::listings::access(state, pool, viewer.as_ref(), &d, id).await;
    if d.kind != "model3d" || !seen || !may || !super::open_to(state, viewer.as_ref()).await {
        return Err(not_found());
    }
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT file_key, file_name FROM mf_model_files WHERE id = $1 AND design_id = $2 AND format = 'scad'",
    )
    .bind(file)
    .bind(id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((key, name)) = row else {
        return Err(not_found());
    };
    let text = match storage.get(&key).await {
        Ok(Some(bytes)) => String::from_utf8(bytes.to_vec()).map_err(|_| not_found())?,
        _ => return Err((StatusCode::BAD_GATEWAY, "The model can't be fetched just now.").into_response()),
    };
    Ok((d, name, text))
}

fn field_html(p: &Parameter) -> String {
    let name = format!("p_{}", p.name);
    let input = match &p.kind {
        Kind::Number { min, max, step } => format!(
            r#"<input type="number" name="{name}" value="{v}"{min}{max} step="{step}">"#,
            v = escape(&p.default.shown()),
            min = min.map_or(String::new(), |m| format!(r#" min="{}""#, number(m))),
            max = max.map_or(String::new(), |m| format!(r#" max="{}""#, number(m))),
            step = step.map_or("any".to_string(), number),
        ),
        Kind::Choice(list) => {
            let options: String = list
                .iter()
                .map(|(v, label)| {
                    let shown = v.shown();
                    format!(
                        r#"<option value="{v}"{sel}>{label}</option>"#,
                        v = escape(&shown),
                        sel = if *v == p.default { " selected" } else { "" },
                        label = escape(label)
                    )
                })
                .collect();
            format!(r#"<select name="{name}">{options}</select>"#)
        }
        Kind::Bool => {
            return format!(
                r#"<label class="choice"><input type="checkbox" name="{name}" value="true"{c}> <span>{label}</span></label>"#,
                c = if p.default == Value::Bool(true) { " checked" } else { "" },
                label = escape(if p.description.is_empty() {
                    &p.name
                } else {
                    &p.description
                })
            );
        }
        Kind::Text(max) => format!(
            r#"<input name="{name}" value="{v}" maxlength="{max}">"#,
            v = escape(&p.default.shown())
        ),
        Kind::Vector(n) => format!(
            r#"<input name="{name}" value="{v}" inputmode="decimal"> <small>{n} numbers, separated by commas</small>"#,
            v = escape(&p.default.shown())
        ),
    };
    let label = if p.description.is_empty() {
        escape(&p.name)
    } else {
        format!("{} <small>({})</small>", escape(&p.description), escape(&p.name))
    };
    format!(r#"<label>{label}{input}</label>"#)
}

fn page_html(id: i64, file: i64, title: &str, name: &str, params: &[Parameter], problem: &str) -> String {
    let mut groups: Vec<&str> = Vec::new();
    for p in params {
        if !groups.contains(&p.group.as_str()) {
            groups.push(&p.group);
        }
    }
    let fields: String = groups
        .iter()
        .map(|g| {
            let inner: String = params.iter().filter(|p| p.group == *g).map(field_html).collect();
            format!(r#"<fieldset><legend>{}</legend>{inner}</fieldset>"#, escape(g))
        })
        .collect();
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem">{}</p>"#, escape(problem))
    };
    let body = if params.is_empty() {
        "<p>This file has no parameters to set.</p>".to_string()
    } else {
        format!(
            r#"<form method="post" action="/d/{id}/customize/{file}">{fields}<button type="submit">Download an STL</button></form>
<p class="muted">Made for you in a minute or less; the model's licence applies to what you download.</p>"#
        )
    };
    format!(
        r#"<section class="form"><h1>Customize {title}</h1><p class="lead">{name}</p>{problem}{body}
<p><a href="/d/{id}/{slug}">Back to the model</a></p></section>"#,
        title = escape(title),
        name = escape(name),
        slug = crate::listings::slug(title)
    )
}

/// `GET /d/<id>/customize/<file>`.
pub async fn page(State(state): State<AppState>, headers: HeaderMap, Path((id, file)): Path<(i64, i64)>) -> Response {
    let (d, name, text) = match source(&state, &headers, id, file).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let params = parameters(&text);
    crate::accounts::private(
        &format!("Customize {} · MakerFoundry", d.title),
        &page_html(id, file, &d.title, &name, &params, ""),
        current_user(&state, &headers).await.is_some(),
    )
}

/// `POST /d/<id>/customize/<file>`: the values checked, the STL made by the render service and
/// read by our own reader, then sent as a download. Signed-in members only, within their limit.
pub async fn make(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, file)): Path<(i64, i64)>,
    axum::Form(form): axum::Form<HashMap<String, String>>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let Some(user) = current_user(&state, &headers).await else {
        return Redirect::to(&format!("/signin?next=/d/{id}/customize/{file}")).into_response();
    };
    let (d, name, text) = match source(&state, &headers, id, file).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let params = parameters(&text);
    let again = |why: &str| {
        crate::accounts::private(
            &format!("Customize {} · MakerFoundry", d.title),
            &page_html(id, file, &d.title, &name, &params, why),
            true,
        )
    };
    let values = match values(&params, &form) {
        Ok(v) => v,
        Err(why) => return again(&format!("{why}.")),
    };
    if !state
        .limits
        .allow(crate::limits::SCAD_RENDER, &user.account_id.to_string())
    {
        return again("You've made a lot of models this hour. Please wait a while.");
    }
    let Some(renderer) = state.scad.as_ref() else {
        return crate::designs::not_found();
    };
    let stl = match renderer.render(&text, &values).await {
        Ok(stl) => stl,
        Err(e) => {
            eprintln!("model3d: customize {id}: {e}");
            return again("The model couldn't be made with these values. Please try others, or try again later.");
        }
    };
    let checked = stl.clone();
    match tokio::task::spawn_blocking(move || super::stl::read(&checked)).await {
        Ok(Ok(_)) => {}
        _ => return again("The model couldn't be made with these values. Please try others, or try again later."),
    }
    let stem = crate::listings::slug(name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s));
    (
        [
            (header::CONTENT_TYPE, "model/stl".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{stem}-custom.stl\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        stl,
    )
        .into_response()
}

#[doc(hidden)]
/// A stand-in render service for tests: answers a cube `size` mm across (the `size` parameter,
/// 10 without it) for a request with the key `test-key`; `fail = true` gets an error. Keeps each
/// request's JSON.
pub mod stand_in {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;

    pub type Calls = Arc<Mutex<Vec<serde_json::Value>>>;

    pub async fn start() -> std::io::Result<(String, Calls)> {
        let calls: Calls = Arc::default();
        async fn render(State(calls): State<Calls>, headers: HeaderMap, body: String) -> (StatusCode, Vec<u8>) {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(v.clone());
            if headers
                .get("authorization")
                .is_none_or(|a| a.as_bytes() != b"Bearer test-key")
            {
                return (StatusCode::UNAUTHORIZED, Vec::new());
            }
            if v["parameters"]["fail"] == serde_json::json!(true) {
                return (StatusCode::UNPROCESSABLE_ENTITY, Vec::new());
            }
            #[allow(clippy::cast_possible_truncation)]
            let size = v["parameters"]["size"].as_f64().unwrap_or(10.0) as f32;
            (StatusCode::OK, super::super::step::stand_in::cube(size))
        }
        let app = Router::new().route("/render", post(render)).with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((base, calls))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: &str = r#"// A box with a lid
// Outer size (mm)
size = 40; // [10:5:100]
// Wall thickness
wall = 2; // [1:4]
// Lid style
lid = "snap"; // [snap, slide, none]
// Corner radius
corners = 3; // [0:Sharp, 3:Round, 6:Very round]
label = "MINE";
hinged = false;
offset = [1, 2, 3];
/* [Text] */
// Font size
font = 8;
/* [Hidden] */
$fn = 64;
secret = 1;
module box() {
  cube(size);
}
after = 5;
"#;

    #[test]
    fn the_customizer_syntax_is_read() {
        let p = parameters(BOX);
        let names: Vec<&str> = p.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["size", "wall", "lid", "corners", "label", "hinged", "offset", "font"]
        );
        assert_eq!(p[0].description, "Outer size (mm)");
        assert_eq!(
            p[0].kind,
            Kind::Number {
                min: Some(10.0),
                max: Some(100.0),
                step: Some(5.0)
            }
        );
        assert_eq!(p[0].default, Value::Number(40.0));
        assert_eq!(
            p[1].kind,
            Kind::Number {
                min: Some(1.0),
                max: Some(4.0),
                step: None
            }
        );
        assert_eq!(
            p[2].kind,
            Kind::Choice(vec![
                (Value::Text("snap".into()), "snap".into()),
                (Value::Text("slide".into()), "slide".into()),
                (Value::Text("none".into()), "none".into()),
            ])
        );
        assert!(matches!(&p[3].kind, Kind::Choice(c) if c[1] == (Value::Number(3.0), "Round".into())));
        assert_eq!(p[4].kind, Kind::Text(200));
        assert_eq!(p[5].kind, Kind::Bool);
        assert_eq!(p[6].kind, Kind::Vector(3));
        assert_eq!((p[7].group.as_str(), p[7].description.as_str()), ("Text", "Font size"));
        assert_eq!(p[0].group, "Parameters");
    }

    #[test]
    fn expressions_and_odd_lines_are_left_out() {
        let p = parameters(
            "a = 1 + 2;\nb = str(\"x\");\nc = [1, 2, 3, 4, 5];\nd = \"a\\\"b\";\n2bad = 3;\ne = 1e999;\nf = 7; // [9:1]\ng = 5;",
        );
        let names: Vec<&str> = p.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["g"]);
        let many: String = (0..500).map(|i| format!("p{i} = {i};\n")).collect();
        assert_eq!(parameters(&many).len(), MAX_PARAMETERS);
    }

    #[test]
    fn form_values_are_checked_against_their_declarations() {
        let p = parameters(BOX);
        let form = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs.iter().map(|(k, v)| (format!("p_{k}"), v.to_string())).collect()
        };
        let v = values(
            &p,
            &form(&[
                ("size", "55"),
                ("lid", "slide"),
                ("corners", "6"),
                ("hinged", "true"),
                ("offset", "4, 5, 6"),
            ]),
        )
        .unwrap();
        assert_eq!(v[0], ("size".into(), Value::Number(55.0)));
        assert_eq!(v[1], ("wall".into(), Value::Number(2.0)), "left out: its default");
        assert_eq!(v[2].1, Value::Text("slide".into()));
        assert_eq!(v[3].1, Value::Number(6.0));
        assert_eq!(v[5].1, Value::Bool(true));
        assert_eq!(v[6].1, Value::Vector(vec![4.0, 5.0, 6.0]));
        for bad in [
            ("size", "500"),
            ("size", "1e3"),
            ("size", "NaN"),
            ("size", "40; import(\"x\")"),
            ("lid", "hinge"),
            ("corners", "4"),
            ("label", "a\"b"),
            ("label", "a\\b"),
            ("offset", "1, 2"),
        ] {
            assert!(values(&p, &form(&[bad])).is_err(), "{bad:?}");
        }
        let unknown = values(&p, &form(&[("secret", "2")])).unwrap();
        assert!(!unknown.iter().any(|(n, _)| n == "secret" || n == "$fn"));
    }

    #[tokio::test]
    async fn the_render_service_gets_only_checked_values() {
        let (base, calls) = stand_in::start().await.unwrap();
        let r = Renderer::new(&base, Some("test-key")).unwrap();
        let stl = r
            .render("size = 10;", &[("size".into(), Value::Number(20.0))])
            .await
            .unwrap();
        assert_eq!(super::super::stl::read(&stl).unwrap().size_mm(), [20.0, 20.0, 20.0]);
        assert!(r.render("x", &[("fail".into(), Value::Bool(true))]).await.is_err());
        assert!(Renderer::new(&base, None).unwrap().render("x", &[]).await.is_err());
        let log = calls.lock().unwrap();
        assert_eq!(log[0]["scad"], "size = 10;");
        assert_eq!(log[0]["parameters"]["size"], 20.0);
        assert!(Renderer::new("file:///x", None).is_none());
        assert_eq!(format!("{r:?}"), "Renderer");
    }
}
