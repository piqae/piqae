//! Reproducible, capability-free renderer load probe.
use piqae_document_renderer::{
    DocumentSpecV1, Node, Page, PageSize, RenderLimits, SPEC_VERSION, TextValue, render,
};
use serde_json::json;
use std::{
    env,
    process::ExitCode,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Configuration {
    iterations: usize,
    concurrency: usize,
    warmup: usize,
    max_p95_ms: Option<f64>,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("render load probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let config = configuration()?;
    let (spec, input) = fixture();
    for _ in 0..config.warmup {
        render(&spec, &input, RenderLimits::default()).map_err(|error| error.to_string())?;
    }
    let (spec, input) = (Arc::new(spec), Arc::new(input));
    let started = Instant::now();
    let workers: Vec<_> = (0..config.concurrency)
        .map(|worker| {
            let (spec, input) = (Arc::clone(&spec), Arc::clone(&input));
            let (iterations, concurrency) = (config.iterations, config.concurrency);
            thread::spawn(move || -> Result<Vec<Duration>, String> {
                let mut samples = Vec::with_capacity(iterations.div_ceil(concurrency));
                for index in (worker..iterations).step_by(concurrency) {
                    let sample_started = Instant::now();
                    let pdf = render(&spec, &input, RenderLimits::default())
                        .map_err(|error| format!("iteration {index}: {error}"))?;
                    let render_elapsed = sample_started.elapsed();
                    if !pdf.starts_with(b"%PDF-1.7") {
                        return Err(format!("iteration {index}: invalid PDF signature"));
                    }
                    samples.push(render_elapsed);
                }
                Ok(samples)
            })
        })
        .collect();
    let mut samples = Vec::with_capacity(config.iterations);
    for worker in workers {
        samples.extend(
            worker
                .join()
                .map_err(|_| "load worker panicked".to_owned())??,
        );
    }
    if samples.len() != config.iterations {
        return Err(format!(
            "sample count mismatch: expected {}, got {}",
            config.iterations,
            samples.len()
        ));
    }
    let elapsed = started.elapsed();
    samples.sort_unstable();
    let p50 = percentile(&samples, 50);
    let p95 = percentile(&samples, 95);
    let p99 = percentile(&samples, 99);
    let passed = config
        .max_p95_ms
        .is_none_or(|threshold| millis(p95) <= threshold);
    println!("{}", serde_json::to_string_pretty(&json!({
        "schema_version":"piqae.render-load-evidence/v1", "scope":"in_process_renderer_only",
        "fixture":"a4_invoice_100_rows_qr", "iterations":config.iterations, "concurrency":config.concurrency,
        "warmup_iterations":config.warmup, "elapsed_ms":millis(elapsed),
        "throughput_documents_per_second":f64::from(u32::try_from(config.iterations).map_err(|_| "iteration count exceeds evidence format".to_owned())?) / elapsed.as_secs_f64(),
        "latency_ms":{"p50":millis(p50),"p95":millis(p95),"p99":millis(p99),"max":millis(*samples.last().unwrap_or(&Duration::ZERO))},
        "thresholds":{"max_p95_ms":config.max_p95_ms}, "passed":passed
    })).map_err(|error| error.to_string())?);
    Ok(passed)
}

fn configuration() -> Result<Configuration, String> {
    let threshold = env::var("PIQAE_RENDER_LOAD_MAX_P95_MS")
        .ok()
        .map(|value| {
            value
                .parse::<f64>()
                .map_err(|_| "PIQAE_RENDER_LOAD_MAX_P95_MS must be a number".to_owned())
        })
        .transpose()?;
    if threshold.is_some_and(|value| !value.is_finite() || value <= 0.0) {
        return Err("PIQAE_RENDER_LOAD_MAX_P95_MS must be finite and positive".into());
    }
    Ok(Configuration {
        iterations: number("PIQAE_RENDER_LOAD_ITERATIONS", 500, 1, 1_000_000)?,
        concurrency: number("PIQAE_RENDER_LOAD_CONCURRENCY", 4, 1, 32)?,
        warmup: number("PIQAE_RENDER_LOAD_WARMUP", 20, 0, 10_000)?,
        max_p95_ms: threshold,
    })
}

fn number(name: &str, default: usize, minimum: usize, maximum: usize) -> Result<usize, String> {
    let value = env::var(name).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| format!("{name} must be an integer"))
    })?;
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}"));
    }
    Ok(value)
}
fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    samples[(samples.len() * percentile).div_ceil(100).saturating_sub(1)]
}
fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn fixture() -> (DocumentSpecV1, serde_json::Value) {
    let mut body = vec![
        Node::Text {
            value: TextValue::Literal("INVOICE".into()),
            font_size: 20.0,
        },
        Node::Text {
            value: TextValue::Binding {
                pointer: "/order/name".into(),
            },
            font_size: 12.0,
        },
        Node::Line,
        Node::Repeat {
            pointer: "/items".into(),
            children: vec![Node::Row {
                children: vec![
                    Node::Text {
                        value: TextValue::Binding {
                            pointer: "./sku".into(),
                        },
                        font_size: 9.0,
                    },
                    Node::Text {
                        value: TextValue::Binding {
                            pointer: "./title".into(),
                        },
                        font_size: 9.0,
                    },
                    Node::Text {
                        value: TextValue::Binding {
                            pointer: "./price".into(),
                        },
                        font_size: 9.0,
                    },
                ],
                gap_mm: 2.0,
            }],
        },
    ];
    body.push(Node::Qr {
        value: TextValue::Binding {
            pointer: "/order/status_url".into(),
        },
        size_mm: 24.0,
    });
    let items:Vec<_>=(0..100).map(|index|json!({"sku":format!("SKU-{index:04}"),"title":format!("Fixture item {index}"),"price":"12.50"})).collect();
    (
        DocumentSpecV1 {
            spec_version: SPEC_VERSION.into(),
            page: Page {
                size: PageSize::A4,
                margin_mm: 10.0,
            },
            body,
        },
        json!({"order":{"name":"#PIQAE-FIXTURE","status_url":"https://example.invalid/orders/fixture"},"items":items}),
    )
}
