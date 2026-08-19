use piqae_document_renderer::{
    BusinessDocumentV1, RenderLimits, ResolvedResources, render_with_metrics,
};
use serde_json::{Value, json};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let order_count = std::env::args()
        .nth(1)
        .as_deref()
        .unwrap_or("250")
        .parse::<usize>()?
        .clamp(1, 1_000);
    let iterations = std::env::args()
        .nth(2)
        .as_deref()
        .unwrap_or("20")
        .parse::<usize>()?
        .clamp(1, 1_000);
    let specification: BusinessDocumentV1 = serde_json::from_value(specification())?;
    let input = input(order_count);
    let input_bytes = serde_json::to_vec(&input)?.len();
    let mut samples = Vec::with_capacity(iterations);
    let mut pdf_bytes = 0;
    let mut pages = 0;
    for _ in 0..iterations {
        let started = Instant::now();
        let output = render_with_metrics(
            &specification,
            &input,
            &ResolvedResources::default(),
            RenderLimits::default(),
        )?;
        samples.push(started.elapsed().as_micros());
        pdf_bytes = output.pdf.len();
        pages = output.page_count;
    }
    samples.sort_unstable();
    let percentile = |percent: usize| samples[(samples.len() - 1) * percent / 100];
    let transfer_reduction_ratio =
        f64::from(u32::try_from(pdf_bytes)?) / f64::from(u32::try_from(input_bytes)?);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "orders": order_count,
            "iterations": iterations,
            "pages": pages,
            "input_bytes": input_bytes,
            "pdf_bytes": pdf_bytes,
            "transfer_reduction_ratio": transfer_reduction_ratio,
            "render_p50_us": percentile(50),
            "render_p95_us": percentile(95),
            "render_p99_us": percentile(99)
        }))?
    );
    Ok(())
}

fn input(order_count: usize) -> Value {
    let orders = (1..=order_count)
        .map(|order| {
            json!({
                "name": format!("#{order:05}"),
                "customer": format!("Customer {order}"),
                "items": (1..=5).map(|line| json!({
                    "title": format!("Product {line}"),
                    "sku": format!("SKU-{line:03}"),
                    "quantity": line
                })).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    json!({ "orders": orders })
}

fn specification() -> Value {
    let style = json!({});
    json!({
        "format": "piqae.business-document/v1",
        "media": {
            "kind": "paged",
            "size": "a4",
            "orientation": "portrait",
            "margins": { "top_mm": 10.0, "right_mm": 10.0, "bottom_mm": 10.0, "left_mm": 10.0 }
        },
        "body": [{
            "type": "repeat",
            "items": { "type": "path", "path": ["orders"] },
            "gap_mm": 0.0,
            "children": [
                { "type": "heading", "level": 1, "style": style, "content": [
                    { "type": "text", "value": "Packing slip ", "style": {} },
                    { "type": "value", "value": { "type": "current_path", "path": ["name"] }, "style": {} }
                ]},
                { "type": "paragraph", "style": {}, "content": [
                    { "type": "text", "value": "Ship to: ", "style": { "bold": true } },
                    { "type": "value", "value": { "type": "current_path", "path": ["customer"] }, "style": {} }
                ]},
                { "type": "table", "items": { "type": "current_path", "path": ["items"] }, "repeat_header": true,
                  "columns": [
                    { "header": [{ "type": "text", "value": "Item", "style": { "bold": true }}], "cell": [{ "type": "value", "value": { "type": "current_path", "path": ["title"] }, "style": {} }], "width": 3.0, "align": "left" },
                    { "header": [{ "type": "text", "value": "SKU", "style": { "bold": true }}], "cell": [{ "type": "value", "value": { "type": "current_path", "path": ["sku"] }, "style": {} }], "width": 2.0, "align": "left" },
                    { "header": [{ "type": "text", "value": "Qty", "style": { "bold": true }}], "cell": [{ "type": "value", "value": { "type": "current_path", "path": ["quantity"] }, "style": {} }], "width": 1.0, "align": "right" }
                  ]
                },
                { "type": "page_break" }
            ]
        }]
    })
}
