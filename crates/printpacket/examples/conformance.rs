use printpacket::{
    OutputTarget, RenderLimits, ResolvedResources, analyze_document, render_cache_key,
    render_with_metrics,
};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

fn main() {
    for (name, fixture) in [
        (
            "receipt-80mm",
            include_str!("../../../standards/printpacket/conformance/receipt-80mm.json"),
        ),
        (
            "production-label-100x50",
            include_str!("../../../standards/printpacket/conformance/production-label-100x50.json"),
        ),
        (
            "invoice-a4",
            include_str!("../../../standards/printpacket/conformance/invoice-a4.json"),
        ),
    ] {
        let fixture: Value =
            serde_json::from_str(fixture).unwrap_or_else(|error| panic!("{name} JSON: {error}"));
        let (document, manifest) = analyze_document(&fixture["template"], RenderLimits::default())
            .unwrap_or_else(|error| panic!("{name} template: {error}"));
        let output = render_with_metrics(
            &document,
            &fixture["data"],
            &ResolvedResources::default(),
            RenderLimits::default(),
        )
        .unwrap_or_else(|error| panic!("{name} render: {error}"));
        let cache = render_cache_key(&manifest, &fixture["data"], &OutputTarget::pdf_v1())
            .unwrap_or_else(|error| panic!("{name} cache: {error}"));
        println!(
            "{name} template={} cache={} pdf={} bytes={} pages={}",
            manifest.canonical_sha256,
            cache,
            format_args!("{:x}", Sha256::digest(&output.pdf)),
            output.pdf.len(),
            output.page_count
        );
    }
}
