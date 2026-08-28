use printpacket_renderer::{PrintPacketV1, RenderLimits, ResolvedResources, render_with_metrics};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn fixture_data() -> Value {
    serde_json::from_str(include_str!(
        "../../../apps/shopify/tests/fixtures/printpacket/normalized-order-data.json"
    ))
    .unwrap_or_else(|error| panic!("Shopify normalized-data fixture must be valid JSON: {error}"))
}

fn sparse_order_data() -> Value {
    serde_json::from_str(include_str!(
        "../../../apps/shopify/tests/fixtures/printpacket/normalized-order-data-sparse.json"
    ))
    .unwrap_or_else(|error| panic!("sparse Shopify order fixture must be valid JSON: {error}"))
}

fn sparse_draft_order_data() -> Value {
    serde_json::from_str(include_str!(
        "../../../apps/shopify/tests/fixtures/printpacket/normalized-draft-order-data-sparse.json"
    ))
    .unwrap_or_else(|error| panic!("sparse Shopify draft fixture must be valid JSON: {error}"))
}

fn render_with_data(specification: Value, data: &Value) -> (u32, Vec<u8>) {
    let packet: PrintPacketV1 = serde_json::from_value(specification)
        .unwrap_or_else(|error| panic!("Shopify PrintPacket fixture must deserialize: {error}"));
    let output = render_with_metrics(
        &packet,
        data,
        &ResolvedResources::default(),
        RenderLimits::default(),
    )
    .unwrap_or_else(|error| panic!("Shopify PrintPacket fixture must render: {error}"));
    (output.page_count, output.pdf)
}

fn render(specification: Value) -> (u32, Vec<u8>) {
    render_with_data(specification, &fixture_data())
}

fn starter_specifications() -> BTreeMap<String, Value> {
    serde_json::from_str(include_str!(
        "../../../apps/shopify/tests/fixtures/printpacket/starter-specifications.json"
    ))
    .unwrap_or_else(|error| panic!("Shopify starter fixture must be valid JSON: {error}"))
}

fn path(parts: &[&str]) -> Value {
    json!({ "type": "path", "path": parts })
}

fn current(parts: &[&str]) -> Value {
    json!({ "type": "current_path", "path": parts })
}

fn money(amount: &[&str], currency: &[&str]) -> Value {
    json!({
        "type": "format_money",
        "amount": current(amount),
        "currency": current(currency)
    })
}

fn paragraph(value: &Value) -> Value {
    json!({ "type": "paragraph", "content": [{ "type": "value", "value": value }] })
}

fn table(with_money: bool) -> Value {
    let mut columns = vec![json!({
        "header": [{ "type": "text", "value": "Item" }],
        "cell": [{ "type": "value", "value": current(&["title"]) }],
        "width": 4
    })];
    if with_money {
        columns.push(json!({
            "header": [{ "type": "text", "value": "Total" }],
            "cell": [{ "type": "value", "value": money(&["total"], &["currency"]) }],
            "width": 2,
            "align": "right"
        }));
    }
    json!({
        "type": "table",
        "items": current(&["lineItems"]),
        "columns": columns,
        "repeat_header": true
    })
}

fn packet(media: &Value, body: &[Value]) -> Value {
    json!({
        "format": "printpacket/v1",
        "media": media,
        "theme": { "font_size_pt": 10, "line_height": 1.25 },
        "resources": {},
        "body": body
    })
}

#[test]
fn canonical_renderer_accepts_the_exact_shopify_starter_packets() {
    let specifications = starter_specifications();
    assert_eq!(
        specifications
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["invoice", "packing-slip", "product-label", "receipt"]
    );
    for (name, specification) in specifications {
        let (pages, pdf) = render(specification);
        assert!(pages > 0, "{name} must produce at least one page");
        assert!(pdf.starts_with(b"%PDF-1.4"), "{name} must produce a PDF");
    }
}

#[test]
fn exact_shopify_starters_render_sparse_regular_and_draft_orders() {
    for (fixture_name, data) in [
        ("regular", sparse_order_data()),
        ("draft", sparse_draft_order_data()),
    ] {
        for (starter_name, specification) in starter_specifications() {
            let (pages, pdf) = render_with_data(specification, &data);
            assert!(
                pages > 0,
                "{starter_name} must render the sparse {fixture_name} fixture"
            );
            assert!(
                pdf.starts_with(b"%PDF-1.4"),
                "{starter_name} must produce a PDF for sparse {fixture_name} data"
            );
        }
    }
}

#[test]
fn editor_generated_packet_renders_single_bulk_and_sparse_draft_data() {
    let specification: Value = serde_json::from_str(include_str!(
        "../../../apps/shopify/tests/fixtures/printpacket/editor-generated-packet.json"
    ))
    .unwrap_or_else(|error| panic!("editor-generated packet fixture must be valid JSON: {error}"));
    let single = fixture_data();
    let mut bulk = single.clone();
    let sparse_order = sparse_order_data();
    bulk["orders"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("bulk fixture orders must be an array"))
        .extend(
            sparse_order["orders"]
                .as_array()
                .unwrap_or_else(|| panic!("sparse fixture orders must be an array"))
                .iter()
                .cloned(),
        );
    for (fixture_name, data) in [
        ("single", single),
        ("bulk", bulk),
        ("draft", sparse_draft_order_data()),
    ] {
        let (pages, pdf) = render_with_data(specification.clone(), &data);
        assert!(pages > 0, "editor packet must render {fixture_name} data");
        assert!(pdf.starts_with(b"%PDF-1.4"));
    }
}

#[test]
fn product_label_renders_only_normalized_code128_safe_candidates() {
    let specification = starter_specifications()
        .remove("product-label")
        .unwrap_or_else(|| panic!("product label starter must exist"));
    for (fixture_name, expected_fallback, data) in [
        ("regular", "SKU-FALLBACK", sparse_order_data()),
        ("draft", "DRAFT-SKU", sparse_draft_order_data()),
    ] {
        let lines = data["orders"][0]["lineItems"]
            .as_array()
            .unwrap_or_else(|| panic!("{fixture_name} line items must be an array"));
        assert_eq!(lines[0]["labelCode128"], json!(expected_fallback));
        assert_eq!(lines[1]["labelCode128"], Value::Null);
        assert_eq!(lines[2]["labelCode128"], Value::Null);
        assert_eq!(lines[3]["labelCode128"], Value::Null);
        let (pages, pdf) = render_with_data(specification.clone(), &data);
        assert!(pages > 0, "product label must render {fixture_name} data");
        assert!(pdf.starts_with(b"%PDF-1.4"));
    }
}

#[test]
fn canonical_renderer_accepts_shopify_invoice_data() {
    let specification = packet(
        &json!({ "kind": "paged", "size": "a4" }),
        &[json!({
            "type": "repeat",
            "items": path(&["orders"]),
            "children": [
                paragraph(&current(&["name"])),
                table(true),
                paragraph(&money(&["total"], &["currency"]))
            ]
        })],
    );
    let (pages, pdf) = render(specification);
    assert_eq!(pages, 1);
    assert!(pdf.starts_with(b"%PDF-1.4"));
}

#[test]
fn canonical_renderer_accepts_shopify_packing_slip_data() {
    let specification = packet(
        &json!({ "kind": "paged", "size": "a4" }),
        &[json!({
            "type": "repeat",
            "items": path(&["orders"]),
            "children": [
                paragraph(&current(&["shippingAddress", "formatted"])),
                { "type": "qr", "value": current(&["statusUrl"]), "size_mm": 24 },
                table(false)
            ]
        })],
    );
    let (pages, pdf) = render(specification);
    assert_eq!(pages, 1);
    assert!(pdf.len() > 1_000);
}

#[test]
fn canonical_renderer_accepts_shopify_receipt_data() {
    let specification = packet(
        &json!({ "kind": "continuous", "width_mm": 80 }),
        &[json!({
            "type": "repeat",
            "items": path(&["orders"]),
            "children": [
                paragraph(&current(&["name"])),
                table(true),
                paragraph(&money(&["tax"], &["currency"])),
                paragraph(&money(&["total"], &["currency"]))
            ]
        })],
    );
    let (pages, pdf) = render(specification);
    assert_eq!(pages, 1);
    assert!(pdf.starts_with(b"%PDF-1.4"));
}

#[test]
fn canonical_renderer_accepts_shopify_product_label_data() {
    let specification = packet(
        &json!({ "kind": "label", "width_mm": 100, "height_mm": 50 }),
        &[json!({
            "type": "repeat",
            "items": path(&["orders"]),
            "children": [{
                "type": "repeat",
                "items": current(&["lineItems"]),
                "children": [
                    paragraph(&current(&["title"])),
                    {
                        "type": "barcode",
                        "value": current(&["sku"]),
                        "symbology": "code128",
                        "width_mm": 70,
                        "height_mm": 12,
                        "human_readable": true
                    }
                ]
            }]
        })],
    );
    let (pages, pdf) = render(specification);
    assert_eq!(pages, 1);
    assert!(pdf.len() > 1_000);
}
