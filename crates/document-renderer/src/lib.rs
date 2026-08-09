//! Deterministic, provider-neutral PDF document rendering.
//!
//! The renderer is deliberately capability-free: it accepts in-memory templates
//! and JSON, performs no I/O, and emits PDF bytes. `DocumentSpecV1` is the stable
//! boundary; the compact PDF backend can be replaced without changing callers.

use std::fmt::Write as _;

use qrcode::{EcLevel, QrCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const SPEC_VERSION: &str = "piqae.document/v1";
/// Exact native renderer implementation version persisted with conversions.
pub const RENDERER_VERSION: &str = concat!("piqae-document-renderer/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Copy)]
pub struct RenderLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_repeat_items: usize,
    pub max_pages: usize,
    pub max_text_bytes: usize,
    pub max_output_bytes: usize,
}
impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_nodes: 10_000,
            max_depth: 32,
            max_repeat_items: 1_000,
            max_pages: 200,
            max_text_bytes: 1_000_000,
            max_output_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("unsupported document spec version: {0}")]
    UnsupportedVersion(String),
    #[error("resource limit exceeded: {0}")]
    Limit(&'static str),
    #[error("invalid JSON pointer: {0}")]
    InvalidPointer(String),
    #[error("QR value is too large")]
    QrTooLarge,
    #[error("page has no printable area")]
    InvalidPage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSpecV1 {
    pub spec_version: String,
    pub page: Page,
    #[serde(default)]
    pub body: Vec<Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub size: PageSize,
    #[serde(default = "default_margin")]
    pub margin_mm: f32,
}

const fn default_margin() -> f32 {
    10.0
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageSize {
    A4,
    A5,
    Letter,
    FourBySix,
    Roll58mm,
    Roll80mm,
}

impl PageSize {
    const fn points(self) -> (f32, f32) {
        match self {
            Self::A4 => (595.28, 841.89),
            Self::A5 => (419.53, 595.28),
            Self::Letter => (612.0, 792.0),
            Self::FourBySix => (288.0, 432.0),
            Self::Roll58mm => (164.41, 841.89),
            Self::Roll80mm => (226.77, 841.89),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Text {
        value: TextValue,
        #[serde(default = "default_font_size")]
        font_size: f32,
    },
    Stack {
        children: Vec<Node>,
        #[serde(default)]
        gap_mm: f32,
    },
    Row {
        children: Vec<Node>,
        #[serde(default)]
        gap_mm: f32,
    },
    Spacer {
        height_mm: f32,
    },
    Line,
    PageBreak,
    When {
        pointer: String,
        children: Vec<Node>,
    },
    Repeat {
        pointer: String,
        children: Vec<Node>,
    },
    Qr {
        value: TextValue,
        #[serde(default = "default_qr_size")]
        size_mm: f32,
    },
    /// A bounded, flow-layout table. Cell values are resolved relative to each
    /// array item; no HTML, script, font, file, or network capability exists.
    Table {
        pointer: String,
        columns: Vec<TableColumn>,
        #[serde(default = "default_font_size")]
        font_size: f32,
        #[serde(default)]
        header: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableColumn {
    pub heading: String,
    pub pointer: String,
    #[serde(default = "default_column_weight")]
    pub width_weight: f32,
}

const fn default_column_weight() -> f32 {
    1.0
}

const fn default_font_size() -> f32 {
    10.0
}
const fn default_qr_size() -> f32 {
    24.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextValue {
    Literal(String),
    Binding { pointer: String },
}

#[derive(Debug, Clone)]
enum Draw {
    Text {
        x: f32,
        y: f32,
        size: f32,
        text: String,
    },
    Line {
        x1: f32,
        y: f32,
        x2: f32,
    },
    Qr {
        x: f32,
        y: f32,
        size: f32,
        modules: Vec<Vec<bool>>,
    },
}

#[derive(Debug)]
struct State {
    pages: Vec<Vec<Draw>>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    margin: f32,
    nodes: usize,
    text_bytes: usize,
    repeated: usize,
}

/// Renders a v1 specification and JSON input to deterministic PDF bytes.
///
/// # Errors
///
/// Returns [`RenderError`] when the spec is invalid, a binding cannot be
/// resolved, QR data cannot be represented, or a configured limit is exceeded.
pub fn render(
    spec: &DocumentSpecV1,
    input: &Value,
    limits: RenderLimits,
) -> Result<Vec<u8>, RenderError> {
    if spec.spec_version != SPEC_VERSION {
        return Err(RenderError::UnsupportedVersion(spec.spec_version.clone()));
    }
    let (width, height) = spec.page.size.points();
    let margin = spec.page.margin_mm * 72.0 / 25.4;
    if !margin.is_finite() || margin < 0.0 || width <= margin * 2.0 || height <= margin * 2.0 {
        return Err(RenderError::InvalidPage);
    }
    let mut state = State {
        pages: vec![Vec::new()],
        x: margin,
        y: height - margin,
        width,
        height,
        margin,
        nodes: 0,
        text_bytes: 0,
        repeated: 0,
    };
    layout(&spec.body, input, input, &mut state, limits, 0)?;
    let pdf = write_pdf(&state);
    if pdf.len() > limits.max_output_bytes {
        return Err(RenderError::Limit("output bytes"));
    }
    Ok(pdf)
}

#[allow(
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::suboptimal_flops
)]
fn layout(
    nodes: &[Node],
    root: &Value,
    current: &Value,
    state: &mut State,
    limits: RenderLimits,
    depth: usize,
) -> Result<(), RenderError> {
    if depth > limits.max_depth {
        return Err(RenderError::Limit("nesting depth"));
    }
    for node in nodes {
        state.nodes += 1;
        if state.nodes > limits.max_nodes {
            return Err(RenderError::Limit("nodes"));
        }
        match node {
            Node::Text { value, font_size } => {
                let text = resolve_text(value, root, current)?;
                draw_text(state, &text, *font_size, limits)?;
            }
            Node::Spacer { height_mm } => {
                ensure_space(state, mm(*height_mm), limits)?;
                state.y -= mm(*height_mm).max(0.0);
            }
            Node::Line => {
                ensure_space(state, 4.0, limits)?;
                state
                    .pages
                    .last_mut()
                    .ok_or(RenderError::Limit("pages"))?
                    .push(Draw::Line {
                        x1: state.margin,
                        y: state.y,
                        x2: state.width - state.margin,
                    });
                state.y -= 4.0;
            }
            Node::PageBreak => new_page(state, limits)?,
            Node::Stack { children, gap_mm } => {
                layout(children, root, current, state, limits, depth + 1)?;
                state.y -= mm(*gap_mm).max(0.0);
            }
            Node::Row { children, gap_mm } => {
                let start_y = state.y;
                let available = state.width - state.margin * 2.0;
                let count = children.len().max(1) as f32;
                let cell = (available - mm(*gap_mm).max(0.0) * (count - 1.0)) / count;
                let mut low_y = start_y;
                for (index, child) in children.iter().enumerate() {
                    state.x = state.margin + index as f32 * (cell + mm(*gap_mm).max(0.0));
                    state.y = start_y;
                    layout(
                        std::slice::from_ref(child),
                        root,
                        current,
                        state,
                        limits,
                        depth + 1,
                    )?;
                    low_y = low_y.min(state.y);
                }
                state.x = state.margin;
                state.y = low_y;
            }
            Node::When { pointer, children } => {
                if truthy(resolve(pointer, root, current)?) {
                    layout(children, root, current, state, limits, depth + 1)?;
                }
            }
            Node::Repeat { pointer, children } => {
                if let Some(items) = resolve(pointer, root, current)?.as_array() {
                    state.repeated = state.repeated.saturating_add(items.len());
                    if state.repeated > limits.max_repeat_items {
                        return Err(RenderError::Limit("repeat items"));
                    }
                    for item in items {
                        layout(children, root, item, state, limits, depth + 1)?;
                    }
                }
            }
            Node::Qr { value, size_mm } => {
                let text = resolve_text(value, root, current)?;
                account_text(state, &text, limits)?;
                if !size_mm.is_finite() {
                    return Err(RenderError::Limit("QR size"));
                }
                let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
                    .map_err(|_| RenderError::QrTooLarge)?;
                let size = mm(*size_mm).clamp(mm(10.0), mm(100.0));
                ensure_space(state, size, limits)?;
                let width = code.width();
                let colors = code.into_colors();
                let modules = colors
                    .chunks(width)
                    .map(|row| row.iter().map(|c| c == &qrcode::Color::Dark).collect())
                    .collect();
                state
                    .pages
                    .last_mut()
                    .ok_or(RenderError::Limit("pages"))?
                    .push(Draw::Qr {
                        x: state.x,
                        y: state.y - size,
                        size,
                        modules,
                    });
                state.y -= size + 4.0;
            }
            Node::Table {
                pointer,
                columns,
                font_size,
                header,
            } => {
                if columns.is_empty() || columns.len() > 64 {
                    return Err(RenderError::Limit("table columns"));
                }
                let weights = columns
                    .iter()
                    .map(|column| column.width_weight)
                    .collect::<Vec<_>>();
                if weights
                    .iter()
                    .any(|weight| !weight.is_finite() || *weight <= 0.0)
                {
                    return Err(RenderError::Limit("table column weights"));
                }
                let total_weight: f32 = weights.iter().sum();
                if !total_weight.is_finite() {
                    return Err(RenderError::Limit("table column weights"));
                }
                let width = state.width - state.margin * 2.0;
                let rows = resolve(pointer, root, current)?
                    .as_array()
                    .ok_or_else(|| RenderError::InvalidPointer(pointer.clone()))?;
                state.repeated = state.repeated.saturating_add(rows.len());
                if state.repeated > limits.max_repeat_items {
                    return Err(RenderError::Limit("repeat items"));
                }
                if *header {
                    state.nodes = state.nodes.saturating_add(columns.len());
                    if state.nodes > limits.max_nodes {
                        return Err(RenderError::Limit("nodes"));
                    }
                    draw_table_row(
                        state,
                        columns.iter().map(|column| column.heading.clone()),
                        &weights,
                        total_weight,
                        width,
                        *font_size,
                        limits,
                    )?;
                }
                for row in rows {
                    state.nodes = state.nodes.saturating_add(columns.len());
                    if state.nodes > limits.max_nodes {
                        return Err(RenderError::Limit("nodes"));
                    }
                    let values = columns
                        .iter()
                        .map(|column| resolve(&column.pointer, root, row).map(value_to_text))
                        .collect::<Result<Vec<_>, _>>()?;
                    draw_table_row(
                        state,
                        values,
                        &weights,
                        total_weight,
                        width,
                        *font_size,
                        limits,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

const fn account_text(
    state: &mut State,
    text: &str,
    limits: RenderLimits,
) -> Result<(), RenderError> {
    state.text_bytes = state.text_bytes.saturating_add(text.len());
    if state.text_bytes > limits.max_text_bytes {
        Err(RenderError::Limit("text bytes"))
    } else {
        Ok(())
    }
}

fn draw_text(
    state: &mut State,
    text: &str,
    font_size: f32,
    limits: RenderLimits,
) -> Result<(), RenderError> {
    account_text(state, text, limits)?;
    if !font_size.is_finite() {
        return Err(RenderError::Limit("font size"));
    }
    let size = font_size.clamp(4.0, 96.0);
    ensure_space(state, size * 1.25, limits)?;
    state
        .pages
        .last_mut()
        .ok_or(RenderError::Limit("pages"))?
        .push(Draw::Text {
            x: state.x,
            y: state.y - size,
            size,
            text: text.to_owned(),
        });
    state.y -= size * 1.25;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_table_row(
    state: &mut State,
    values: impl IntoIterator<Item = String>,
    weights: &[f32],
    total_weight: f32,
    width: f32,
    font_size: f32,
    limits: RenderLimits,
) -> Result<(), RenderError> {
    if !font_size.is_finite() {
        return Err(RenderError::Limit("font size"));
    }
    let size = font_size.clamp(4.0, 96.0);
    ensure_space(state, size * 1.65, limits)?;
    let y = state.y - size;
    let mut x = state.margin;
    for (value, weight) in values.into_iter().zip(weights) {
        account_text(state, &value, limits)?;
        state
            .pages
            .last_mut()
            .ok_or(RenderError::Limit("pages"))?
            .push(Draw::Text {
                x,
                y,
                size,
                text: value,
            });
        x += width * *weight / total_weight;
    }
    state.y -= size * 1.35;
    state
        .pages
        .last_mut()
        .ok_or(RenderError::Limit("pages"))?
        .push(Draw::Line {
            x1: state.margin,
            y: state.y,
            x2: state.width - state.margin,
        });
    state.y -= size * 0.3;
    Ok(())
}

fn mm(value: f32) -> f32 {
    if value.is_finite() {
        value * 72.0 / 25.4
    } else {
        0.0
    }
}
fn ensure_space(state: &mut State, required: f32, limits: RenderLimits) -> Result<(), RenderError> {
    if !required.is_finite()
        || required < 0.0
        || required > state.margin.mul_add(-2.0, state.height)
    {
        return Err(RenderError::Limit("element height"));
    }
    if state.y - required < state.margin {
        new_page(state, limits)?;
    }
    Ok(())
}
fn new_page(state: &mut State, limits: RenderLimits) -> Result<(), RenderError> {
    if state.pages.len() >= limits.max_pages {
        return Err(RenderError::Limit("pages"));
    }
    state.pages.push(Vec::new());
    state.x = state.margin;
    state.y = state.height - state.margin;
    Ok(())
}

fn resolve<'a>(
    pointer: &str,
    root: &'a Value,
    current: &'a Value,
) -> Result<&'a Value, RenderError> {
    let (base, path) = if let Some(path) = pointer.strip_prefix("./") {
        (current, format!("/{path}"))
    } else if pointer == "." {
        return Ok(current);
    } else {
        (root, pointer.to_owned())
    };
    if !path.is_empty() && !path.starts_with('/') {
        return Err(RenderError::InvalidPointer(pointer.to_owned()));
    }
    base.pointer(&path)
        .ok_or_else(|| RenderError::InvalidPointer(pointer.to_owned()))
}
fn resolve_text(value: &TextValue, root: &Value, current: &Value) -> Result<String, RenderError> {
    match value {
        TextValue::Literal(v) => Ok(v.clone()),
        TextValue::Binding { pointer } => {
            let value = resolve(pointer, root, current)?;
            Ok(value_to_text(value))
        }
    }
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
        Value::Number(_) => true,
    }
}

fn write_pdf(state: &State) -> Vec<u8> {
    let page_count = state.pages.len();
    let font_id = 3 + page_count * 2;
    let mut objects = vec![String::new(); font_id];
    objects[0] = "<< /Type /Catalog /Pages 2 0 R >>".into();
    let kids = (0..page_count)
        .map(|i| format!("{} 0 R", 3 + i * 2))
        .collect::<Vec<_>>()
        .join(" ");
    objects[1] = format!("<< /Type /Pages /Count {page_count} /Kids [{kids}] >>");
    for (i, draws) in state.pages.iter().enumerate() {
        let page_id = 3 + i * 2;
        let content_id = page_id + 1;
        objects[page_id - 1] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources << /Font << /F1 {font_id} 0 R >> >> /Contents {content_id} 0 R >>",
            state.width, state.height
        );
        let stream = content(draws);
        objects[content_id - 1] = format!(
            "<< /Length {} >>\nstream\n{}endstream",
            stream.len(),
            stream
        );
    }
    objects[font_id - 1] =
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into();
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, object).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]
fn content(draws: &[Draw]) -> String {
    let mut out = String::new();
    for draw in draws {
        match draw {
            Draw::Text { x, y, size, text } => {
                let _ = writeln!(
                    out,
                    "BT /F1 {:.2} Tf {:.2} {:.2} Td ({}) Tj ET",
                    size,
                    x,
                    y,
                    escape_pdf(text)
                );
            }
            Draw::Line { x1, y, x2 } => {
                let _ = writeln!(out, "0.5 w {x1:.2} {y:.2} m {x2:.2} {y:.2} l S");
            }
            Draw::Qr {
                x,
                y,
                size,
                modules,
            } => {
                let module = size / modules.len() as f32;
                out.push_str("0 g\n");
                for (row, values) in modules.iter().enumerate() {
                    for (column, dark) in values.iter().enumerate() {
                        if *dark {
                            let _ = writeln!(
                                out,
                                "{:.3} {:.3} {:.3} {:.3} re f",
                                x + column as f32 * module,
                                y + (modules.len() - row - 1) as f32 * module,
                                module,
                                module
                            );
                        }
                    }
                }
            }
        }
    }
    out
}
fn escape_pdf(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '(' => "\\(".into(),
            ')' => "\\)".into(),
            '\\' => "\\\\".into(),
            '\n' | '\r' => " ".into(),
            c if (' '..='~').contains(&c) => c.to_string(),
            _ => "?".into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec(body: Vec<Node>) -> DocumentSpecV1 {
        DocumentSpecV1 {
            spec_version: SPEC_VERSION.into(),
            page: Page {
                size: PageSize::A4,
                margin_mm: 10.0,
            },
            body,
        }
    }
    #[test]
    fn output_is_a_deterministic_pdf() {
        let doc = spec(vec![Node::Text {
            value: TextValue::Binding {
                pointer: "/name".into(),
            },
            font_size: 12.0,
        }]);
        let input = serde_json::json!({"name":"Invoice (safe)"});
        let one = render(&doc, &input, RenderLimits::default()).unwrap_or_default();
        let two = render(&doc, &input, RenderLimits::default()).unwrap_or_default();
        assert_eq!(one, two);
        assert!(one.starts_with(b"%PDF-1.7"));
        assert!(String::from_utf8_lossy(&one).contains("Invoice \\(safe\\)"));
    }
    #[test]
    fn repeat_is_bounded() {
        let doc = spec(vec![Node::Repeat {
            pointer: "/items".into(),
            children: vec![Node::Text {
                value: TextValue::Binding {
                    pointer: ".".into(),
                },
                font_size: 10.0,
            }],
        }]);
        let err = render(
            &doc,
            &serde_json::json!({"items":[1,2,3]}),
            RenderLimits {
                max_repeat_items: 2,
                ..RenderLimits::default()
            },
        );
        assert_eq!(err, Err(RenderError::Limit("repeat items")));
    }
    #[test]
    fn pages_and_nodes_are_bounded() {
        let doc = spec(vec![Node::PageBreak, Node::PageBreak]);
        assert_eq!(
            render(
                &doc,
                &Value::Null,
                RenderLimits {
                    max_pages: 2,
                    ..RenderLimits::default()
                }
            ),
            Err(RenderError::Limit("pages"))
        );
        let doc = spec(vec![Node::Line]);
        assert_eq!(
            render(
                &doc,
                &Value::Null,
                RenderLimits {
                    max_nodes: 0,
                    ..RenderLimits::default()
                }
            ),
            Err(RenderError::Limit("nodes"))
        );
        let doc = spec(vec![Node::Stack {
            children: vec![Node::Stack {
                children: vec![Node::Line],
                gap_mm: 0.0,
            }],
            gap_mm: 0.0,
        }]);
        assert_eq!(
            render(
                &doc,
                &Value::Null,
                RenderLimits {
                    max_depth: 1,
                    ..RenderLimits::default()
                }
            ),
            Err(RenderError::Limit("nesting depth"))
        );
    }
    #[test]
    fn rejects_bad_version_and_pointer() {
        let mut doc = spec(Vec::new());
        doc.spec_version = "other".into();
        assert!(matches!(
            render(&doc, &Value::Null, RenderLimits::default()),
            Err(RenderError::UnsupportedVersion(_))
        ));
        let doc = spec(vec![Node::Text {
            value: TextValue::Binding {
                pointer: "relative".into(),
            },
            font_size: 10.0,
        }]);
        assert!(matches!(
            render(&doc, &Value::Null, RenderLimits::default()),
            Err(RenderError::InvalidPointer(_))
        ));
    }
    #[test]
    fn renders_qr_without_external_io() {
        let doc = spec(vec![Node::Qr {
            value: TextValue::Literal("https://piqae.com/jobs/1".into()),
            size_mm: 24.0,
        }]);
        let pdf = render(&doc, &Value::Null, RenderLimits::default()).unwrap_or_default();
        assert!(String::from_utf8_lossy(&pdf).contains(" re f"));
    }
    #[test]
    fn serde_rejects_unknown_capabilities() {
        let json = r#"{"spec_version":"piqae.document/v1","page":{"size":"a4"},"body":[],"remote_url":"https://example.com"}"#;
        assert!(serde_json::from_str::<DocumentSpecV1>(json).is_err());
    }

    #[test]
    fn table_is_bounded_and_uses_item_relative_pointers() {
        let doc = spec(vec![Node::Table {
            pointer: "/items".into(),
            columns: vec![
                TableColumn {
                    heading: "SKU".into(),
                    pointer: "./sku".into(),
                    width_weight: 1.0,
                },
                TableColumn {
                    heading: "Qty".into(),
                    pointer: "./quantity".into(),
                    width_weight: 1.0,
                },
            ],
            font_size: 9.0,
            header: true,
        }]);
        let pdf = render(
            &doc,
            &serde_json::json!({"items":[{"sku":"A-1","quantity":2}]}),
            RenderLimits::default(),
        )
        .unwrap_or_default();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("SKU"));
        assert!(text.contains("A-1"));
        assert_eq!(
            render(
                &doc,
                &serde_json::json!({"items":[{},{}]}),
                RenderLimits {
                    max_repeat_items: 1,
                    ..RenderLimits::default()
                }
            ),
            Err(RenderError::Limit("repeat items"))
        );
    }

    #[test]
    fn rejects_non_finite_and_overheight_programmatic_values() {
        let nan_text = spec(vec![Node::Text {
            value: TextValue::Literal("x".into()),
            font_size: f32::NAN,
        }]);
        assert_eq!(
            render(&nan_text, &Value::Null, RenderLimits::default()),
            Err(RenderError::Limit("font size"))
        );
        let nan_qr = spec(vec![Node::Qr {
            value: TextValue::Literal("x".into()),
            size_mm: f32::NAN,
        }]);
        assert_eq!(
            render(&nan_qr, &Value::Null, RenderLimits::default()),
            Err(RenderError::Limit("QR size"))
        );
        let huge_spacer = spec(vec![Node::Spacer {
            height_mm: 10_000.0,
        }]);
        assert_eq!(
            render(&huge_spacer, &Value::Null, RenderLimits::default()),
            Err(RenderError::Limit("element height"))
        );
    }

    #[test]
    fn arbitrary_text_cannot_inject_pdf_operators() {
        for value in [") Tj ET\n0 0 999 999 re f\nBT (", "\\()\r\n", "\0\u{7f}🔥"] {
            let doc = spec(vec![Node::Text {
                value: TextValue::Literal(value.into()),
                font_size: 10.0,
            }]);
            let pdf = render(&doc, &Value::Null, RenderLimits::default()).unwrap_or_default();
            let body = String::from_utf8_lossy(&pdf);
            assert!(!body.contains("\n0 0 999 999 re f"));
            assert_eq!(
                body.lines().filter(|line| line.starts_with("BT ")).count(),
                1
            );
        }
    }

    #[test]
    fn all_low_unicode_scalars_preserve_one_text_operation() {
        // Deterministic property-style coverage for control bytes, PDF
        // delimiters, Latin text, combining marks, and non-WinAnsi input.
        for scalar in (0..=0x2ff).filter_map(char::from_u32) {
            let value = format!("prefix{scalar}()\\\r\nsuffix");
            let doc = spec(vec![Node::Text {
                value: TextValue::Literal(value),
                font_size: 10.0,
            }]);
            let one = render(&doc, &Value::Null, RenderLimits::default()).unwrap_or_default();
            let two = render(&doc, &Value::Null, RenderLimits::default()).unwrap_or_default();
            assert_eq!(one, two);
            let body = String::from_utf8_lossy(&one);
            assert_eq!(
                body.lines().filter(|line| line.starts_with("BT ")).count(),
                1
            );
        }
    }

    #[test]
    fn qr_payload_counts_towards_the_shared_text_budget() {
        let doc = spec(vec![Node::Qr {
            value: TextValue::Literal("12345".into()),
            size_mm: 20.0,
        }]);
        assert_eq!(
            render(
                &doc,
                &Value::Null,
                RenderLimits {
                    max_text_bytes: 4,
                    ..RenderLimits::default()
                }
            ),
            Err(RenderError::Limit("text bytes"))
        );
    }
}
