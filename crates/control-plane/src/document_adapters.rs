//! Bounded, data-only hosted adapter conversions.
//!
//! This module intentionally does not link pdfme, execute plugins or JavaScript,
//! open files, or perform network I/O.

use serde::Serialize;
use serde_json::{Value, json};

pub const PDFME_ADAPTER_VERSION: &str = "1.1.0";
pub const PDFME_LEGACY_ADAPTER_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: &'static str,
    pub path: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Converted {
    pub document: Value,
    pub warnings: Vec<Diagnostic>,
    pub fidelity: &'static str,
}

fn error(
    code: &'static str,
    path: impl Into<String>,
    message: impl Into<String>,
    feature: Option<&'static str>,
) -> Diagnostic {
    Diagnostic {
        code,
        severity: "error",
        path: path.into(),
        message: message.into(),
        feature,
    }
}

fn pointer(name: &str) -> String {
    format!("/{}", name.replace('~', "~0").replace('/', "~1"))
}

fn page_size(base: &serde_json::Map<String, Value>) -> Option<&'static str> {
    let (w, h) = (base.get("width")?.as_f64()?, base.get("height")?.as_f64()?);
    if !w.is_finite() || !h.is_finite() {
        return None;
    }
    let close = |a: f64, b: f64| (a - b).abs() <= 0.6;
    if close(w, 210.0) && close(h, 297.0) {
        Some("a4")
    } else if close(w, 148.0) && close(h, 210.0) {
        Some("a5")
    } else if close(w, 215.9) && close(h, 279.4) {
        Some("letter")
    } else if close(w, 101.6) && close(h, 152.4) {
        Some("four-by-six")
    } else if close(w, 58.0) && (1.0..=2000.0).contains(&h) {
        Some("roll58mm")
    } else if close(w, 80.0) && (1.0..=2000.0).contains(&h) {
        Some("roll80mm")
    } else {
        None
    }
}

/// Converts the documented pdfme JSON subset without invoking a runtime.
///
/// # Errors
///
/// Returns stable diagnostics when input is structurally invalid, unsupported,
/// or would be lossy while strict conversion is requested.
#[allow(clippy::too_many_lines)]
pub fn convert_pdfme(source: &Value, _strict: bool) -> Result<Converted, Vec<Diagnostic>> {
    let Some(template) = source.as_object() else {
        return Err(vec![error(
            "PDFME_INVALID_TEMPLATE",
            "$",
            "Template must be a JSON object.",
            None,
        )]);
    };
    let Some(base) = template.get("basePdf").and_then(Value::as_object) else {
        return Err(vec![error(
            "PDFME_BASE_PDF_UNSUPPORTED",
            "$.basePdf",
            "Only a blank object basePdf is convertible.",
            Some("base-pdf"),
        )]);
    };
    let Some(size) = page_size(base) else {
        return Err(vec![error(
            "PDFME_PAGE_SIZE_UNSUPPORTED",
            "$.basePdf",
            "Blank page dimensions do not match a Piqae page preset.",
            Some("blank-page"),
        )]);
    };
    // `page_size` has already established that both dimensions are finite.
    // Retain the source dimensions so every accepted absolute box is also
    // wholly contained by the page that the renderer will create.
    let page_width_mm = base.get("width").and_then(Value::as_f64).unwrap_or(0.0);
    let page_height_mm = base.get("height").and_then(Value::as_f64).unwrap_or(0.0);
    let Some(pages) = template.get("schemas").and_then(Value::as_array) else {
        return Err(vec![error(
            "PDFME_SCHEMAS_REQUIRED",
            "$.schemas",
            "schemas must be an array of page arrays.",
            None,
        )]);
    };
    let mut body = Vec::new();
    let warnings = Vec::new();
    let mut errors = Vec::new();
    for (page_index, page) in pages.iter().enumerate() {
        let Some(schemas) = page.as_array() else {
            errors.push(error(
                "PDFME_PAGE_INVALID",
                format!("$.schemas[{page_index}]"),
                "Page schemas must be an array.",
                None,
            ));
            continue;
        };
        if page_index > 0 {
            body.push(json!({"type":"page_break"}));
        }
        let mut canvas = Vec::new();
        for (source_index, raw_schema) in schemas.iter().enumerate() {
            let path = format!("$.schemas[{page_index}][{source_index}]");
            let Some(schema) = raw_schema.as_object() else {
                errors.push(error(
                    "PDFME_SCHEMA_INVALID",
                    &path,
                    "Schema must be an object.",
                    None,
                ));
                continue;
            };
            let position = schema.get("position").and_then(Value::as_object);
            let finite =
                |value: Option<&Value>| value.and_then(Value::as_f64).filter(|v| v.is_finite());
            let (Some(x), Some(y), Some(width), Some(height)) = (
                finite(position.and_then(|p| p.get("x"))),
                finite(position.and_then(|p| p.get("y"))),
                finite(schema.get("width")),
                finite(schema.get("height")),
            ) else {
                errors.push(error(
                    "PDFME_BOX_REQUIRED",
                    &path,
                    "Supported schemas require finite position.x, position.y, width and height.",
                    Some("absolute-positioning"),
                ));
                continue;
            };
            if x < 0.0
                || y < 0.0
                || width <= 0.0
                || height <= 0.0
                || x > 2000.0
                || y > 2000.0
                || width > 2000.0
                || height > 2000.0
                || x + width > page_width_mm
                || y + height > page_height_mm
            {
                errors.push(error(
                    "PDFME_BOX_INVALID",
                    &path,
                    "Schema box must be positive, bounded, and contained by the page.",
                    Some("absolute-positioning"),
                ));
                continue;
            }
            let value = schema
                .get("name")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(|v| json!({"pointer":pointer(v)}))
                .or_else(|| {
                    schema
                        .get("content")
                        .and_then(Value::as_str)
                        .map(|v| json!(v))
                });
            match schema.get("type").and_then(Value::as_str) {
                Some("text" | "qrcode") if value.is_none() => errors.push(error(
                    "PDFME_VALUE_REQUIRED", &path, "Text and QR schemas require a name or literal string content.", None)),
                Some("text") => canvas.push(json!({"type":"text","value":value,"x_mm":x,"y_mm":y,"width_mm":width,"height_mm":height,"font_size":schema.get("fontSize").and_then(Value::as_f64).filter(|v|v.is_finite()).unwrap_or(10.0)})),
                Some("qrcode") => canvas.push(json!({"type":"qr","value":value,"x_mm":x,"y_mm":y,"width_mm":width,"height_mm":height})),
                Some("line") => canvas.push(json!({"type":"line","x_mm":x,"y_mm":y,"width_mm":width,"height_mm":height})),
                Some(other) => errors.push(error("PDFME_SCHEMA_TYPE_UNSUPPORTED", format!("{path}.type"), format!("Schema type {other:?} is unsupported because hosted conversion never runs pdfme plugins."), Some("plugins"))),
                None => errors.push(error("PDFME_SCHEMA_TYPE_UNSUPPORTED", format!("{path}.type"), "Schema type unknown is unsupported because hosted conversion never runs pdfme plugins.", Some("plugins"))),
            }
        }
        body.push(json!({"type":"canvas","children":canvas}));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Converted {
        document: json!({"spec_version":"piqae.document/v1","page":{"size":size,"margin_mm":0},"body":body}),
        fidelity: "exact",
        warnings,
    })
}

/// Preserves the original flow-reduction semantics for persisted 1.0.0 clients.
///
/// # Errors
///
/// Returns the stable 1.0.0 diagnostics for invalid, unsupported or strictly
/// lossy input.
#[allow(clippy::too_many_lines)]
pub fn convert_pdfme_legacy(source: &Value, strict: bool) -> Result<Converted, Vec<Diagnostic>> {
    let Some(template) = source.as_object() else {
        return Err(vec![error(
            "PDFME_INVALID_TEMPLATE",
            "$",
            "Template must be a JSON object.",
            None,
        )]);
    };
    let Some(base) = template.get("basePdf").and_then(Value::as_object) else {
        return Err(vec![error(
            "PDFME_BASE_PDF_UNSUPPORTED",
            "$.basePdf",
            "Only a blank object basePdf is convertible.",
            Some("base-pdf"),
        )]);
    };
    let Some(size) = page_size(base) else {
        return Err(vec![error(
            "PDFME_PAGE_SIZE_UNSUPPORTED",
            "$.basePdf",
            "Blank page dimensions do not match a Piqae page preset.",
            Some("blank-page"),
        )]);
    };
    let Some(pages) = template.get("schemas").and_then(Value::as_array) else {
        return Err(vec![error(
            "PDFME_SCHEMAS_REQUIRED",
            "$.schemas",
            "schemas must be an array of page arrays.",
            None,
        )]);
    };
    let mut body = Vec::new();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    for (page_index, page) in pages.iter().enumerate() {
        let Some(schemas) = page.as_array() else {
            errors.push(error(
                "PDFME_PAGE_INVALID",
                format!("$.schemas[{page_index}]"),
                "Page schemas must be an array.",
                None,
            ));
            continue;
        };
        if page_index > 0 {
            body.push(json!({"type":"page_break"}));
        }
        let mut ordered = schemas
            .iter()
            .enumerate()
            .filter_map(|(index, value)| value.as_object().map(|object| (index, object)))
            .collect::<Vec<_>>();
        ordered.sort_by(|left, right| {
            let y = |schema: &serde_json::Map<String, Value>| {
                schema
                    .get("position")
                    .and_then(Value::as_object)
                    .and_then(|position| position.get("y"))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
            };
            y(left.1).total_cmp(&y(right.1))
        });
        let mut cursor = 0.0_f64;
        for (source_index, schema) in ordered {
            let path = format!("$.schemas[{page_index}][{source_index}]");
            let position = schema.get("position").and_then(Value::as_object);
            let y = position
                .and_then(|value| value.get("y"))
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .unwrap_or(cursor);
            if y > cursor {
                body.push(json!({"type":"spacer","height_mm":y-cursor}));
            }
            if position.is_some() || schema.contains_key("width") || schema.contains_key("height") {
                let diagnostic = Diagnostic { code: "PDFME_LAYOUT_LOSSY", severity: if strict { "error" } else { "warning" }, path: path.clone(), message: "pdfme absolute boxes are reduced to vertical flow; horizontal position, dimensions and overlap are not retained.".into(), feature: Some("absolute-positioning") };
                if strict {
                    errors.push(diagnostic);
                } else {
                    warnings.push(diagnostic);
                }
            }
            let value = schema
                .get("name")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(|value| json!({"pointer":pointer(value)}))
                .or_else(|| {
                    schema
                        .get("content")
                        .and_then(Value::as_str)
                        .map(|value| json!(value))
                });
            let Some(value) = value else {
                errors.push(error(
                    "PDFME_VALUE_REQUIRED",
                    &path,
                    "Schema requires a name or literal string content.",
                    None,
                ));
                continue;
            };
            match schema.get("type").and_then(Value::as_str) {
                Some("text") => body.push(json!({"type":"text","value":value,"font_size":schema.get("fontSize").and_then(Value::as_f64).filter(|value|value.is_finite()).unwrap_or(10.0)})),
                Some("qrcode") => body.push(json!({"type":"qr","value":value,"size_mm":schema.get("width").and_then(Value::as_f64).filter(|value|value.is_finite()).unwrap_or(24.0)})),
                Some(other) => errors.push(error("PDFME_SCHEMA_TYPE_UNSUPPORTED", format!("{path}.type"), format!("Schema type {other:?} is unsupported because hosted conversion never runs pdfme plugins."), Some("plugins"))),
                None => errors.push(error("PDFME_SCHEMA_TYPE_UNSUPPORTED", format!("{path}.type"), "Schema type unknown is unsupported because hosted conversion never runs pdfme plugins.", Some("plugins"))),
            }
            cursor = cursor.max(
                y + schema
                    .get("height")
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite())
                    .unwrap_or(0.0),
            );
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Converted {
        document: json!({"spec_version":"piqae.document/v1","page":{"size":size,"margin_mm":0},"body":body}),
        fidelity: if warnings.is_empty() {
            "exact"
        } else {
            "lossy"
        },
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converts_without_execution() {
        let source = json!({"basePdf":{"width":210,"height":297},"schemas":[[{"type":"text","name":"order/name","position":{"x":10,"y":20},"width":80,"height":10}]]});
        let Ok(result) = convert_pdfme(&source, true) else {
            panic!("supported conversion failed")
        };
        assert_eq!(
            result.document["body"][0]["children"][0]["value"]["pointer"],
            "/order~1name"
        );
    }
    #[test]
    fn absolute_layout_is_exact_and_ordered() {
        let source = json!({"basePdf":{"width":210,"height":297},"schemas":[[
            {"type":"text","content":"first","position":{"x":50,"y":90},"width":20,"height":10},
            {"type":"line","content":"ignored","position":{"x":1,"y":2},"width":100,"height":1}
        ]]});
        let Ok(converted) = convert_pdfme(&source, true) else {
            panic!("exact conversion failed")
        };
        assert_eq!(converted.fidelity, "exact");
        assert_eq!(converted.document["body"][0]["children"][0]["x_mm"], 50.0);
        assert_eq!(converted.document["body"][0]["children"][1]["type"], "line");
    }

    #[test]
    fn roll_height_is_finite_positive_and_bounded() {
        for height in [0.0, -1.0, 2001.0, f64::INFINITY, f64::NAN] {
            let source = json!({"basePdf":{"width":58,"height":height},"schemas":[]});
            let Err(errors) = convert_pdfme(&source, true) else {
                panic!("invalid roll height accepted")
            };
            assert_eq!(errors[0].code, "PDFME_PAGE_SIZE_UNSUPPORTED");
        }
        let source = json!({"basePdf":{"width":80,"height":500},"schemas":[]});
        let Ok(converted) = convert_pdfme(&source, true) else {
            panic!("valid roll rejected")
        };
        assert_eq!(converted.document["page"]["size"], "roll80mm");
    }

    #[test]
    fn rejects_boxes_that_cross_page_edges() {
        for schema in [
            json!({"type":"text","content":"x","position":{"x":205,"y":10},"width":10,"height":10}),
            json!({"type":"qrcode","content":"x","position":{"x":10,"y":292},"width":10,"height":10}),
        ] {
            let source = json!({"basePdf":{"width":210,"height":297},"schemas":[[schema]]});
            let Err(errors) = convert_pdfme(&source, true) else {
                panic!("off-page box accepted")
            };
            assert_eq!(errors[0].code, "PDFME_BOX_INVALID");
        }
    }
}
