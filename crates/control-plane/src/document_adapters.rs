//! Bounded, data-only hosted adapter conversions.
//!
//! This module intentionally does not link pdfme, execute plugins or JavaScript,
//! open files, or perform network I/O.

use serde::Serialize;
use serde_json::{Value, json};

pub const PDFME_ADAPTER_VERSION: &str = "1.0.0";

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
pub fn convert_pdfme(source: &Value, strict: bool) -> Result<Converted, Vec<Diagnostic>> {
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
        let mut ordered: Vec<(usize, &serde_json::Map<String, Value>)> = schemas
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.as_object().map(|o| (i, o)))
            .collect();
        ordered.sort_by(|a, b| {
            let y = |v: &serde_json::Map<String, Value>| {
                v.get("position")
                    .and_then(Value::as_object)
                    .and_then(|p| p.get("y"))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
            };
            y(a.1).total_cmp(&y(b.1))
        });
        let mut cursor = 0.0_f64;
        for (source_index, schema) in ordered {
            let path = format!("$.schemas[{page_index}][{source_index}]");
            let position = schema.get("position").and_then(Value::as_object);
            let y = position
                .and_then(|p| p.get("y"))
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .unwrap_or(cursor);
            if y > cursor {
                body.push(json!({"type":"spacer","height_mm":y-cursor}));
            }
            if position.is_some() || schema.contains_key("width") || schema.contains_key("height") {
                let diagnostic = Diagnostic { code:"PDFME_LAYOUT_LOSSY", severity:if strict{"error"}else{"warning"}, path:path.clone(), message:"pdfme absolute boxes are reduced to vertical flow; horizontal position, dimensions and overlap are not retained.".into(), feature:Some("absolute-positioning") };
                if strict {
                    errors.push(diagnostic);
                } else {
                    warnings.push(diagnostic);
                }
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
                Some("text") => body.push(json!({"type":"text","value":value,"font_size":schema.get("fontSize").and_then(Value::as_f64).filter(|v|v.is_finite()).unwrap_or(10.0)})),
                Some("qrcode") => body.push(json!({"type":"qr","value":value,"size_mm":schema.get("width").and_then(Value::as_f64).filter(|v|v.is_finite()).unwrap_or(24.0)})),
                Some(other) => errors.push(error("PDFME_SCHEMA_TYPE_UNSUPPORTED", format!("{path}.type"), format!("Schema type {other:?} is unsupported because hosted conversion never runs pdfme plugins."), Some("plugins"))),
                None => errors.push(error("PDFME_SCHEMA_TYPE_UNSUPPORTED", format!("{path}.type"), "Schema type unknown is unsupported because hosted conversion never runs pdfme plugins.", Some("plugins"))),
            }
            cursor = cursor.max(
                y + schema
                    .get("height")
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite())
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
        let source = json!({"basePdf":{"width":210,"height":297},"schemas":[[{"type":"text","name":"order/name"}]]});
        let Ok(result) = convert_pdfme(&source, true) else {
            panic!("supported conversion failed")
        };
        assert_eq!(
            result.document["body"][0]["value"]["pointer"],
            "/order~1name"
        );
    }
    #[test]
    fn strict_rejects_lossy_layout() {
        let source = json!({"basePdf":{"width":210,"height":297},"schemas":[[{"type":"text","content":"x","position":{"x":1,"y":2}}]]});
        let Err(errors) = convert_pdfme(&source, true) else {
            panic!("strict accepted lossy")
        };
        assert_eq!(errors[0].code, "PDFME_LAYOUT_LOSSY");
        let Ok(converted) = convert_pdfme(&source, false) else {
            panic!("explicit lossy failed")
        };
        assert_eq!(converted.fidelity, "lossy");
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
}
