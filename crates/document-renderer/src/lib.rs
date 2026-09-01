//! Capability-free reference renderer for the vendor-neutral `PrintPacket` format.
//!
//! The crate deliberately performs no file, font, or network I/O. It accepts a
//! bounded semantic document and JSON data and produces deterministic PDF bytes.
//! Renderer ABI v1 uses deterministic PDF Base-14 Helvetica with Windows-1252
//! encoding. Characters outside that exact profile fail explicitly rather than
//! being substituted. A later embedded-font ABI can extend scripts without
//! changing the packet semantics.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::semicolon_if_nothing_returned,
    clippy::suboptimal_flops,
    clippy::too_many_lines
)]

use std::{collections::BTreeMap, fmt::Write as _};

use qrcode::{EcLevel, QrCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// Canonical vendor-neutral `PrintPacket` identifier for new documents.
pub const PRINT_PACKET_DOCUMENT_FORMAT: &str = "printpacket/v1";
pub const RENDERER_VERSION: &str =
    concat!("printpacket-reference-renderer/", env!("CARGO_PKG_VERSION"));
const PAGE_NUMBER_MARKER: &str = "\u{e000}\u{e000}\u{e000}\u{e000}";
const PAGE_COUNT_MARKER: &str = "\u{e001}\u{e001}\u{e001}\u{e001}";

#[derive(Debug, Clone, Copy)]
pub struct RenderLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_expression_nodes: usize,
    pub max_expression_depth: usize,
    pub max_expression_values: usize,
    pub max_path_segments: usize,
    pub max_path_segment_bytes: usize,
    pub max_literal_bytes: usize,
    pub max_repeat_items: usize,
    pub max_table_columns: usize,
    pub max_pages: usize,
    pub max_text_bytes: usize,
    pub max_output_bytes: usize,
    pub max_total_resource_bytes: usize,
    pub max_continuous_height_mm: f32,
}
impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_nodes: 20_000,
            max_depth: 32,
            max_expression_nodes: 50_000,
            max_expression_depth: 64,
            max_expression_values: 100,
            max_path_segments: 64,
            max_path_segment_bytes: 120,
            max_literal_bytes: 1024 * 1024,
            // A Shopify bulk run can contain 250 orders with many line items.
            // Keep the work strictly bounded while allowing that production
            // case without splitting it into slower independent renders.
            max_repeat_items: 20_000,
            max_table_columns: 32,
            max_pages: 1_000,
            max_text_bytes: 1_000_000,
            max_output_bytes: 50 * 1024 * 1024,
            max_total_resource_bytes: 12 * 1024 * 1024,
            max_continuous_height_mm: 2_000.0,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("unsupported PrintPacket format: {0}")]
    UnsupportedVersion(String),
    #[error("invalid document: {0}")]
    Invalid(&'static str),
    #[error("resource limit exceeded: {0}")]
    Limit(&'static str),
    #[error("expression path was not found: {0}")]
    MissingPath(String),
    #[error("expression type mismatch: {0}")]
    Expression(&'static str),
    #[error("unsupported feature: {0}")]
    Unsupported(&'static str),
    #[error("unsupported character U+{code:04X} in Base-14 typography profile")]
    UnsupportedCharacter { code: u32 },
    #[error("QR payload cannot be encoded")]
    QrTooLarge,
    #[error("invalid barcode value: {0}")]
    InvalidBarcode(&'static str),
}

/// Canonical public `PrintPacket` model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintPacketV1 {
    pub format: String,
    pub media: Media,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub resources: BTreeMap<String, Resource>,
    #[serde(default)]
    pub header: Option<Region>,
    #[serde(default)]
    pub body: Vec<Node>,
    #[serde(default)]
    pub footer: Option<Region>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Media {
    Paged {
        size: PageSize,
        #[serde(default)]
        orientation: Orientation,
        #[serde(default = "default_margins")]
        margins: Edges,
    },
    Continuous {
        width_mm: f32,
        #[serde(default = "default_margins")]
        margins: Edges,
    },
    Label {
        width_mm: f32,
        height_mm: f32,
        #[serde(default = "default_margins")]
        margins: Edges,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageSize {
    A4,
    A5,
    Letter,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edges {
    pub top_mm: f32,
    pub right_mm: f32,
    pub bottom_mm: f32,
    pub left_mm: f32,
}
const fn default_margins() -> Edges {
    Edges {
        top_mm: 10.0,
        right_mm: 10.0,
        bottom_mm: 10.0,
        left_mm: 10.0,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    #[serde(default = "default_font_size")]
    pub font_size_pt: f32,
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    #[serde(default)]
    pub text_color: Color,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            font_size_pt: 10.0,
            line_height: 1.25,
            text_color: Color::default(),
        }
    }
}
const fn default_font_size() -> f32 {
    10.0
}
const fn default_line_height() -> f32 {
    1.25
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Resource {
    /// Content-addressed assets are resolved by the host before rendering. This
    /// renderer intentionally rejects image nodes until bytes are supplied by a
    /// future capability-free embedded-resource ABI.
    Image {
        digest: String,
        media_type: String,
        byte_length: u64,
    },
}

/// Verified, in-memory asset bytes supplied by the host. The renderer never
/// fetches assets and verifies these bytes against the published resource
/// digest before use.
#[derive(Debug, Clone, Default)]
pub struct ResolvedResources {
    pub images: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    #[serde(default)]
    pub first: Vec<Node>,
    #[serde(default)]
    pub default: Vec<Node>,
    #[serde(default)]
    pub last: Vec<Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Section {
        children: Vec<Node>,
        #[serde(default)]
        gap_mm: f32,
    },
    Box {
        children: Vec<Node>,
        #[serde(default)]
        style: BoxStyle,
    },
    Paragraph {
        content: Vec<Inline>,
        #[serde(default)]
        style: TextStyle,
    },
    Heading {
        content: Vec<Inline>,
        #[serde(default = "default_heading_level")]
        level: u8,
        #[serde(default)]
        style: TextStyle,
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
    Grid {
        columns: Vec<f32>,
        children: Vec<Node>,
        #[serde(default)]
        gap_mm: f32,
    },
    Table {
        items: Expr,
        columns: Vec<TableColumn>,
        #[serde(default)]
        repeat_header: bool,
        #[serde(default)]
        empty: Vec<Node>,
        #[serde(default)]
        style: TableStyle,
    },
    Repeat {
        items: Expr,
        children: Vec<Node>,
        #[serde(default)]
        gap_mm: f32,
    },
    DataList {
        items: Expr,
        #[serde(default)]
        header: Vec<Node>,
        item: Vec<Node>,
        #[serde(default)]
        empty: Vec<Node>,
        #[serde(default = "default_true")]
        repeat_header: bool,
        #[serde(default)]
        gap_mm: f32,
    },
    Conditional {
        condition: Expr,
        then: Vec<Node>,
        #[serde(rename = "else", default)]
        otherwise: Vec<Node>,
    },
    Spacer {
        height_mm: f32,
    },
    Divider {
        #[serde(default = "default_divider_width")]
        width_pt: f32,
    },
    PageBreak,
    KeepTogether {
        children: Vec<Node>,
    },
    Image {
        resource: String,
        width_mm: f32,
        height_mm: f32,
        #[serde(default)]
        fit: ImageFit,
    },
    ImageValue {
        resource: Expr,
        width_mm: f32,
        height_mm: f32,
        #[serde(default)]
        fit: ImageFit,
    },
    Qr {
        value: Expr,
        size_mm: f32,
        #[serde(default)]
        error_correction: QrCorrection,
    },
    Barcode {
        value: Expr,
        symbology: BarcodeSymbology,
        width_mm: f32,
        height_mm: f32,
        #[serde(default)]
        human_readable: bool,
        #[serde(default)]
        align: TextAlign,
        #[serde(default)]
        padding_mm: f32,
        #[serde(default = "default_barcode_gap")]
        gap_mm: f32,
    },
}
const fn default_heading_level() -> u8 {
    1
}
const fn default_true() -> bool {
    true
}
const fn default_divider_width() -> f32 {
    0.5
}
const fn default_barcode_gap() -> f32 {
    1.4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableColumn {
    pub header: Vec<Inline>,
    pub cell: Vec<Inline>,
    #[serde(default = "default_column_weight")]
    pub width: f32,
    #[serde(default)]
    pub align: TextAlign,
}
const fn default_column_weight() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Inline {
    Text {
        value: String,
        #[serde(default)]
        style: TextStyle,
    },
    Value {
        value: Expr,
        #[serde(default)]
        style: TextStyle,
    },
    LineBreak,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub font_size_pt: Option<f32>,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub color: Option<Color>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoxStyle {
    #[serde(default)]
    pub padding_mm: f32,
    #[serde(default)]
    pub background: Option<Color>,
    #[serde(default)]
    pub border_color: Option<Color>,
    #[serde(default)]
    pub border_width_pt: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableStyle {
    #[serde(default = "default_cell_padding")]
    pub cell_padding_mm: f32,
    #[serde(default)]
    pub header_background: Option<Color>,
    #[serde(default)]
    pub header_text_color: Option<Color>,
    #[serde(default)]
    pub border_color: Option<Color>,
    #[serde(default = "default_table_border_width")]
    pub border_width_pt: f32,
}
impl Default for TableStyle {
    fn default() -> Self {
        Self {
            cell_padding_mm: 1.0,
            header_background: None,
            header_text_color: None,
            border_color: None,
            border_width_pt: 0.25,
        }
    }
}
const fn default_cell_padding() -> f32 {
    1.0
}
const fn default_table_border_width() -> f32 {
    0.25
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    #[default]
    Contain,
    Cover,
    Fill,
    ScaleDown,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum QrCorrection {
    L,
    #[default]
    M,
    Q,
    H,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarcodeSymbology {
    Code128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expr {
    Literal {
        value: Value,
    },
    Path {
        path: Vec<String>,
    },
    CurrentPath {
        path: Vec<String>,
    },
    Coalesce {
        values: Vec<Expr>,
    },
    Concat {
        values: Vec<Expr>,
    },
    Compare {
        operator: CompareOperator,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Boolean {
        operator: BooleanOperator,
        values: Vec<Expr>,
    },
    Not {
        value: Box<Expr>,
    },
    Exists {
        value: Box<Expr>,
    },
    Contains {
        collection: Box<Expr>,
        value: Box<Expr>,
    },
    PageNumber,
    PageCount,
    Arithmetic {
        operator: ArithmeticOperator,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    FormatNumber {
        value: Box<Expr>,
        #[serde(default)]
        decimals: u8,
    },
    FormatMoney {
        amount: Box<Expr>,
        currency: Box<Expr>,
        #[serde(default = "default_money_decimals")]
        decimals: u8,
    },
    FormatDate {
        value: Box<Expr>,
        format: DateFormat,
    },
    FormatString {
        value: Box<Expr>,
        operation: StringOperation,
    },
}
const fn default_money_decimals() -> u8 {
    2
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOperator {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOperator {
    And,
    Or,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArithmeticOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateFormat {
    IsoDate,
    DayMonthYear,
    MonthDayYear,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StringOperation {
    Trim,
    UppercaseAscii,
    LowercaseAscii,
}

#[derive(Debug, Clone)]
enum Draw {
    Text {
        x: f32,
        y: f32,
        size: f32,
        text: String,
        face: FontFace,
        underline: bool,
        color: Color,
    },
    Line {
        x1: f32,
        y: f32,
        x2: f32,
        width: f32,
        color: Color,
    },
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f32,
    },
    Qr {
        x: f32,
        y: f32,
        size: f32,
        modules: Vec<Vec<bool>>,
    },
    Bars {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        bits: Vec<bool>,
    },
    Jpeg {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        pixel_width: u16,
        pixel_height: u16,
        resource_id: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontFace {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}
#[derive(Debug, Clone)]
struct PageDraw {
    width: f32,
    height: f32,
    draws: Vec<Draw>,
    header_draws: usize,
}
struct State<'a> {
    doc: &'a PrintPacketV1,
    limits: RenderLimits,
    root: &'a Value,
    current: Value,
    resolved: &'a ResolvedResources,
    pages: Vec<PageDraw>,
    width: f32,
    nominal_height: f32,
    margins: Edges,
    x: f32,
    y: f32,
    content_width: f32,
    bottom: f32,
    nodes: usize,
    repeats: usize,
    text_bytes: usize,
    continuous: bool,
    in_region: bool,
    pending_page_break: bool,
    estimated_pdf_bytes: usize,
}

/// Render a validated document and its data into deterministic PDF bytes.
///
/// # Errors
/// Returns a stable validation, capability, expression, or resource-limit error.
pub fn render(
    spec: &PrintPacketV1,
    input: &Value,
    limits: RenderLimits,
) -> Result<Vec<u8>, RenderError> {
    render_with_resources(spec, input, &ResolvedResources::default(), limits)
}

/// Render with host-resolved, content-addressed in-memory resources.
///
/// # Errors
/// Returns an error if an asset is absent, has a digest mismatch, is malformed,
/// uses an unsupported media type, or if ordinary rendering fails.
pub fn render_with_resources(
    spec: &PrintPacketV1,
    input: &Value,
    resolved: &ResolvedResources,
    limits: RenderLimits,
) -> Result<Vec<u8>, RenderError> {
    Ok(render_with_metrics(spec, input, resolved, limits)?.pdf)
}

#[derive(Debug, Clone)]
pub struct RenderOutput {
    pub pdf: Vec<u8>,
    pub page_count: u32,
}

/// Render deterministic bytes and authoritative layout metrics in one pass.
///
/// # Errors
/// Returns the same bounded validation, expression, resource, and output errors
/// as [`render_with_resources`].
pub fn render_with_metrics(
    spec: &PrintPacketV1,
    input: &Value,
    resolved: &ResolvedResources,
    limits: RenderLimits,
) -> Result<RenderOutput, RenderError> {
    if !input.is_object() {
        return Err(RenderError::Invalid("render input must be a JSON object"));
    }
    validate(spec, limits)?;
    validate_resolved_resources(spec, resolved)?;
    let (width, height, margins, continuous) = media_geometry(&spec.media, limits)?;
    let mut state = State {
        doc: spec,
        limits,
        root: input,
        current: input.clone(),
        resolved,
        pages: vec![PageDraw {
            width,
            height,
            draws: vec![],
            header_draws: 0,
        }],
        width,
        nominal_height: height,
        margins,
        x: mm(margins.left_mm),
        y: height - mm(margins.top_mm),
        content_width: width - mm(margins.left_mm + margins.right_mm),
        bottom: mm(margins.bottom_mm),
        nodes: 0,
        repeats: 0,
        text_bytes: 0,
        continuous,
        in_region: false,
        pending_page_break: false,
        estimated_pdf_bytes: 4_096_usize
            .checked_add(
                spec.resources
                    .values()
                    .try_fold(0_usize, |total, resource| {
                        let Resource::Image { byte_length, .. } = resource;
                        let length = usize::try_from(*byte_length)
                            .map_err(|_| RenderError::Limit("resource bytes"))?;
                        total
                            .checked_add(length)
                            .ok_or(RenderError::Limit("resource bytes"))
                    })?,
            )
            .ok_or(RenderError::Limit("output bytes"))?,
    };
    if state.estimated_pdf_bytes > limits.max_output_bytes {
        return Err(RenderError::Limit("output bytes"));
    }
    reserve_regions(&mut state)?;
    render_region(&mut state, RegionKind::Header, false)?;
    layout_nodes(&spec.body, &mut state, 0)?;
    replace_final_header(&mut state)?;
    render_region(&mut state, RegionKind::Footer, true)?;
    if continuous {
        let used = (state.nominal_height - state.y + mm(state.margins.bottom_mm)).max(mm(10.0));
        if used > mm(limits.max_continuous_height_mm) {
            return Err(RenderError::Limit("continuous height"));
        }
        let delta = state.nominal_height - used;
        let page = &mut state.pages[0];
        page.height = used;
        for draw in &mut page.draws {
            translate_y(draw, -delta);
        }
    }
    let pdf = write_pdf(&state.pages, resolved, limits.max_output_bytes)?;
    Ok(RenderOutput {
        page_count: u32::try_from(state.pages.len()).map_err(|_| RenderError::Limit("pages"))?,
        pdf,
    })
}

/// Verifies that every declared resource has exact, renderer-compatible bytes.
///
/// Hosts remain responsible for bounded acquisition and tenant authorization;
/// this shared check is the final capability, length, digest, and JPEG-structure
/// gate used by both cloud and node rendering.
///
/// # Errors
/// Returns an error for missing, undeclared, malformed, or mismatched resources.
pub fn validate_resolved_resources(
    spec: &PrintPacketV1,
    resolved: &ResolvedResources,
) -> Result<(), RenderError> {
    if resolved.images.len() != spec.resources.len()
        || resolved
            .images
            .keys()
            .any(|resource_id| !spec.resources.contains_key(resource_id))
    {
        return Err(RenderError::Invalid("resolved resource set mismatch"));
    }
    for (resource_id, resource) in &spec.resources {
        let Resource::Image {
            digest,
            media_type,
            byte_length,
        } = resource;
        if media_type != "image/jpeg" {
            return Err(RenderError::Unsupported(
                "renderer ABI v1 supports resolved JPEG images only",
            ));
        }
        let bytes = resolved
            .images
            .get(resource_id)
            .ok_or(RenderError::Invalid("resolved resource set mismatch"))?;
        if u64::try_from(bytes.len()).ok() != Some(*byte_length) {
            return Err(RenderError::Invalid("image byte length mismatch"));
        }
        let actual = format!("sha256:{:x}", Sha256::digest(bytes));
        if !actual.eq_ignore_ascii_case(digest) {
            return Err(RenderError::Invalid("image digest mismatch"));
        }
        let _ = jpeg_dimensions(bytes)?;
    }
    Ok(())
}

/// Validate a `PrintPacket` without performing rendering or external I/O.
///
/// # Errors
/// Returns the first deterministic schema, capability, or configured-limit error.
pub fn validate(spec: &PrintPacketV1, limits: RenderLimits) -> Result<(), RenderError> {
    if spec.format != PRINT_PACKET_DOCUMENT_FORMAT {
        return Err(RenderError::UnsupportedVersion(spec.format.clone()));
    }
    if !(4.0..=72.0).contains(&spec.theme.font_size_pt)
        || !(1.0..=3.0).contains(&spec.theme.line_height)
    {
        return Err(RenderError::Invalid("theme typography"));
    }
    if spec.resources.len() > 100 {
        return Err(RenderError::Limit("resources"));
    }
    let mut total_resource_bytes = 0_usize;
    for (resource_id, resource) in &spec.resources {
        if resource_id.is_empty() || resource_id.len() > 120 {
            return Err(RenderError::Invalid("resource id"));
        }
        let Resource::Image {
            digest,
            media_type,
            byte_length,
        } = resource;
        let hash = digest.strip_prefix("sha256:").unwrap_or_default();
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RenderError::Invalid("resource digest"));
        }
        if media_type != "image/jpeg" || *byte_length == 0 || *byte_length > 4 * 1024 * 1024 {
            return Err(RenderError::Invalid("resource metadata"));
        }
        total_resource_bytes = total_resource_bytes
            .checked_add(
                usize::try_from(*byte_length).map_err(|_| RenderError::Limit("resource bytes"))?,
            )
            .ok_or(RenderError::Limit("resource bytes"))?;
        if total_resource_bytes > limits.max_total_resource_bytes {
            return Err(RenderError::Limit("resource bytes"));
        }
    }
    let mut count = 0;
    let mut expression_count = 0;
    validate_nodes(&spec.body, 0, &mut count, &mut expression_count, limits)?;
    if let Some(r) = &spec.header {
        validate_nodes(&r.first, 1, &mut count, &mut expression_count, limits)?;
        validate_nodes(&r.default, 1, &mut count, &mut expression_count, limits)?;
        validate_nodes(&r.last, 1, &mut count, &mut expression_count, limits)?;
    }
    if let Some(r) = &spec.footer {
        validate_nodes(&r.first, 1, &mut count, &mut expression_count, limits)?;
        validate_nodes(&r.default, 1, &mut count, &mut expression_count, limits)?;
        validate_nodes(&r.last, 1, &mut count, &mut expression_count, limits)?;
    }
    let region_has_page_break = spec.header.as_ref().is_some_and(|region| {
        contains_page_break(&region.first)
            || contains_page_break(&region.default)
            || contains_page_break(&region.last)
    }) || spec.footer.as_ref().is_some_and(|region| {
        contains_page_break(&region.first)
            || contains_page_break(&region.default)
            || contains_page_break(&region.last)
    });
    if matches!(spec.media, Media::Continuous { .. })
        && (contains_page_break(&spec.body) || region_has_page_break)
    {
        return Err(RenderError::Unsupported("page breaks on continuous media"));
    }
    Ok(())
}

fn contains_page_break(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match node {
        Node::PageBreak => true,
        Node::Section { children, .. }
        | Node::Box { children, .. }
        | Node::Stack { children, .. }
        | Node::Row { children, .. }
        | Node::Grid { children, .. }
        | Node::Repeat { children, .. }
        | Node::KeepTogether { children } => contains_page_break(children),
        Node::DataList {
            header,
            item,
            empty,
            ..
        } => contains_page_break(header) || contains_page_break(item) || contains_page_break(empty),
        Node::Conditional {
            then, otherwise, ..
        } => contains_page_break(then) || contains_page_break(otherwise),
        Node::Table { empty, .. } => contains_page_break(empty),
        _ => false,
    })
}
fn validate_nodes(
    nodes: &[Node],
    depth: usize,
    count: &mut usize,
    expression_count: &mut usize,
    limits: RenderLimits,
) -> Result<(), RenderError> {
    if depth > limits.max_depth {
        return Err(RenderError::Limit("nesting depth"));
    }
    for n in nodes {
        *count += 1;
        if *count > limits.max_nodes {
            return Err(RenderError::Limit("nodes"));
        }
        match n {
            Node::Section { children, gap_mm } | Node::Stack { children, gap_mm } => {
                checked_mm(*gap_mm, "gap")?;
                validate_nodes(children, depth + 1, count, expression_count, limits)?
            }
            Node::Row { children, gap_mm } => {
                checked_mm(*gap_mm, "column gap")?;
                if children.len() > limits.max_table_columns {
                    return Err(RenderError::Limit("row columns"));
                }
                validate_nodes(children, depth + 1, count, expression_count, limits)?
            }
            Node::KeepTogether { children } => {
                validate_nodes(children, depth + 1, count, expression_count, limits)?
            }
            Node::Repeat {
                items,
                children,
                gap_mm,
            } => {
                checked_mm(*gap_mm, "gap")?;
                validate_expr(items, 0, expression_count, limits)?;
                validate_nodes(children, depth + 1, count, expression_count, limits)?
            }
            Node::Box { children, style } => {
                if !style.padding_mm.is_finite()
                    || !(0.0..=50.0).contains(&style.padding_mm)
                    || !style.border_width_pt.is_finite()
                    || !(0.0..=10.0).contains(&style.border_width_pt)
                {
                    return Err(RenderError::Invalid("box style"));
                }
                validate_nodes(children, depth + 1, count, expression_count, limits)?
            }
            Node::Grid {
                columns,
                children,
                gap_mm,
            } => {
                checked_mm(*gap_mm, "column gap")?;
                if columns.is_empty()
                    || columns.len() != children.len()
                    || columns.len() > limits.max_table_columns
                    || columns.iter().any(|v| !v.is_finite() || *v <= 0.0)
                    || !columns.iter().sum::<f32>().is_finite()
                {
                    return Err(RenderError::Invalid("grid columns"));
                }
                validate_nodes(children, depth + 1, count, expression_count, limits)?
            }
            Node::Conditional {
                condition,
                then,
                otherwise,
            } => {
                validate_expr(condition, 0, expression_count, limits)?;
                validate_nodes(then, depth + 1, count, expression_count, limits)?;
                validate_nodes(otherwise, depth + 1, count, expression_count, limits)?
            }
            Node::DataList {
                items,
                header,
                item,
                empty,
                gap_mm,
                ..
            } => {
                if !gap_mm.is_finite() || !(0.0..=100.0).contains(gap_mm) {
                    return Err(RenderError::Invalid("data-list gap"));
                }
                validate_expr(items, 0, expression_count, limits)?;
                validate_nodes(header, depth + 1, count, expression_count, limits)?;
                validate_nodes(item, depth + 1, count, expression_count, limits)?;
                validate_nodes(empty, depth + 1, count, expression_count, limits)?;
            }
            Node::Table {
                items,
                columns,
                empty,
                style,
                ..
            } => {
                if columns.is_empty()
                    || columns.len() > limits.max_table_columns
                    || columns
                        .iter()
                        .any(|c| !c.width.is_finite() || c.width <= 0.0)
                {
                    return Err(RenderError::Invalid("table columns"));
                }
                if !style.cell_padding_mm.is_finite()
                    || !(0.0..=20.0).contains(&style.cell_padding_mm)
                    || !style.border_width_pt.is_finite()
                    || !(0.0..=10.0).contains(&style.border_width_pt)
                {
                    return Err(RenderError::Invalid("table style"));
                }
                validate_expr(items, 0, expression_count, limits)?;
                for column in columns {
                    validate_inlines(&column.header, expression_count, limits)?;
                    validate_inlines(&column.cell, expression_count, limits)?;
                }
                validate_nodes(empty, depth + 1, count, expression_count, limits)?;
            }
            Node::Paragraph { content, style } => {
                validate_text_style(style)?;
                validate_inlines(content, expression_count, limits)?;
            }
            Node::Heading {
                content,
                level,
                style,
            } => {
                if !(1..=6).contains(level) {
                    return Err(RenderError::Invalid("heading level"));
                }
                validate_text_style(style)?;
                validate_inlines(content, expression_count, limits)?;
            }
            Node::Image {
                resource,
                width_mm,
                height_mm,
                fit,
            } => {
                if resource.is_empty() || resource.len() > 120 {
                    return Err(RenderError::Invalid("resource id"));
                }
                validate_image_dimensions(*width_mm, *height_mm, *fit)?;
            }
            Node::ImageValue {
                resource,
                width_mm,
                height_mm,
                fit,
            } => {
                validate_image_dimensions(*width_mm, *height_mm, *fit)?;
                validate_expr(resource, 0, expression_count, limits)?;
            }
            Node::Qr { value, size_mm, .. } => {
                let size = checked_mm(*size_mm, "QR size")?;
                if size < mm(8.0) {
                    return Err(RenderError::Invalid("QR size"));
                }
                validate_expr(value, 0, expression_count, limits)?;
            }
            Node::Barcode {
                value,
                width_mm,
                height_mm,
                padding_mm,
                gap_mm,
                ..
            } => {
                let width = checked_mm(*width_mm, "barcode width")?;
                let height = checked_mm(*height_mm, "barcode height")?;
                if width < mm(20.0) || height < mm(8.0) {
                    return Err(RenderError::InvalidBarcode(
                        "dimensions are below the supported minimum",
                    ));
                }
                checked_mm(*padding_mm, "barcode padding")?;
                checked_mm(*gap_mm, "barcode gap")?;
                if *padding_mm > 50.0 || *gap_mm > 20.0 {
                    return Err(RenderError::Invalid("barcode spacing"));
                }
                validate_expr(value, 0, expression_count, limits)?;
            }
            Node::Spacer { height_mm } => {
                checked_mm(*height_mm, "spacer")?;
            }
            Node::Divider { width_pt } => {
                if !width_pt.is_finite() || !(0.1..=10.0).contains(width_pt) {
                    return Err(RenderError::Invalid("divider width"));
                }
            }
            Node::PageBreak => {}
        }
    }
    Ok(())
}

fn validate_inlines(
    values: &[Inline],
    expression_count: &mut usize,
    limits: RenderLimits,
) -> Result<(), RenderError> {
    for value in values {
        match value {
            Inline::Text { value, .. } => {
                if value.len() > limits.max_literal_bytes {
                    return Err(RenderError::Limit("inline text bytes"));
                }
            }
            Inline::Value { value, style } => {
                validate_text_style(style)?;
                validate_expr(value, 0, expression_count, limits)?;
            }
            Inline::LineBreak => {}
        }
    }
    Ok(())
}

fn validate_text_style(style: &TextStyle) -> Result<(), RenderError> {
    if style
        .font_size_pt
        .is_some_and(|size| !size.is_finite() || !(4.0..=72.0).contains(&size))
    {
        return Err(RenderError::Invalid("text style"));
    }
    Ok(())
}

fn validate_image_dimensions(
    width_mm: f32,
    height_mm: f32,
    fit: ImageFit,
) -> Result<(), RenderError> {
    if matches!(fit, ImageFit::Cover) {
        return Err(RenderError::Unsupported(
            "image cover cropping is not available in renderer ABI v1",
        ));
    }
    if checked_mm(width_mm, "image width")? == 0.0 || checked_mm(height_mm, "image height")? == 0.0
    {
        return Err(RenderError::Invalid("image dimensions"));
    }
    Ok(())
}

fn validate_expr(
    expression: &Expr,
    depth: usize,
    count: &mut usize,
    limits: RenderLimits,
) -> Result<(), RenderError> {
    if depth > limits.max_expression_depth {
        return Err(RenderError::Limit("expression depth"));
    }
    *count = count
        .checked_add(1)
        .ok_or(RenderError::Limit("expression nodes"))?;
    if *count > limits.max_expression_nodes {
        return Err(RenderError::Limit("expression nodes"));
    }
    let nested = |value: &Expr, count: &mut usize| validate_expr(value, depth + 1, count, limits);
    match expression {
        Expr::Literal { value } => {
            if serde_json::to_vec(value)
                .map_or(true, |bytes| bytes.len() > limits.max_literal_bytes)
            {
                return Err(RenderError::Limit("expression literal bytes"));
            }
        }
        Expr::Path { path } | Expr::CurrentPath { path } => {
            if path.len() > limits.max_path_segments
                || path.iter().any(|segment| {
                    segment.is_empty() || segment.len() > limits.max_path_segment_bytes
                })
            {
                return Err(RenderError::Limit("expression path"));
            }
        }
        Expr::Coalesce { values } | Expr::Concat { values } | Expr::Boolean { values, .. } => {
            let maximum = limits.max_expression_values.min(100);
            if values.is_empty() || values.len() > maximum {
                return Err(RenderError::Limit("expression operands"));
            }
            for value in values {
                nested(value, count)?;
            }
        }
        Expr::Compare { left, right, .. }
        | Expr::Arithmetic { left, right, .. }
        | Expr::Contains {
            collection: left,
            value: right,
        } => {
            nested(left, count)?;
            nested(right, count)?;
        }
        Expr::Not { value }
        | Expr::Exists { value }
        | Expr::FormatNumber { value, .. }
        | Expr::FormatDate { value, .. }
        | Expr::FormatString { value, .. } => nested(value, count)?,
        Expr::FormatMoney {
            amount, currency, ..
        } => {
            nested(amount, count)?;
            nested(currency, count)?;
        }
        Expr::PageNumber | Expr::PageCount => {}
    }
    Ok(())
}

fn media_geometry(
    media: &Media,
    limits: RenderLimits,
) -> Result<(f32, f32, Edges, bool), RenderError> {
    let (w, h, m, c) = match media {
        Media::Paged {
            size,
            orientation,
            margins,
        } => {
            let (mut w, mut h) = match size {
                PageSize::A4 => (595.28, 841.89),
                PageSize::A5 => (419.53, 595.28),
                PageSize::Letter => (612.0, 792.0),
            };
            if matches!(orientation, Orientation::Landscape) {
                std::mem::swap(&mut w, &mut h);
            }
            (w, h, *margins, false)
        }
        Media::Continuous { width_mm, margins } => (
            mm(*width_mm),
            mm(limits.max_continuous_height_mm),
            *margins,
            true,
        ),
        Media::Label {
            width_mm,
            height_mm,
            margins,
        } => (mm(*width_mm), mm(*height_mm), *margins, false),
    };
    if !w.is_finite()
        || !h.is_finite()
        || w <= mm(20.0)
        || h <= mm(10.0)
        || [m.top_mm, m.right_mm, m.bottom_mm, m.left_mm]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
        || w <= mm(m.left_mm + m.right_mm)
        || h <= mm(m.top_mm + m.bottom_mm)
    {
        return Err(RenderError::Invalid("media geometry"));
    }
    Ok((w, h, m, c))
}

#[derive(Clone, Copy)]
enum RegionKind {
    Header,
    Footer,
}
fn reserve_regions(state: &mut State) -> Result<(), RenderError> {
    let header = estimate_region(state.doc.header.as_ref(), state)?;
    let footer = estimate_region(state.doc.footer.as_ref(), state)?;
    state.y -= header;
    if state.continuous {
        return Ok(());
    }
    state.bottom += footer;
    if state.y <= state.bottom {
        return Err(RenderError::Invalid("header and footer leave no body area"));
    }
    Ok(())
}
fn estimate_region(region: Option<&Region>, state: &State) -> Result<f32, RenderError> {
    let Some(r) = region else { return Ok(0.0) };
    let height = [&r.first, &r.default, &r.last]
        .into_iter()
        .map(|nodes| {
            estimate_nodes(
                nodes,
                state.content_width,
                state.doc.theme.font_size_pt,
                state.doc.theme.line_height,
            )
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .fold(0.0_f32, f32::max);
    if height > mm(60.0) {
        return Err(RenderError::Limit("header/footer height"));
    }
    Ok(height)
}
fn render_region(state: &mut State, kind: RegionKind, last: bool) -> Result<(), RenderError> {
    let nodes = match kind {
        RegionKind::Header => state.doc.header.as_ref().map(|r| {
            if state.pages.len() == 1 && !r.first.is_empty() {
                &r.first
            } else if last && !r.last.is_empty() {
                &r.last
            } else {
                &r.default
            }
        }),
        RegionKind::Footer => state.doc.footer.as_ref().map(|r| {
            if last && !r.last.is_empty() {
                &r.last
            } else if state.pages.len() == 1 && !r.first.is_empty() {
                &r.first
            } else {
                &r.default
            }
        }),
    };
    let Some(nodes) = nodes.cloned() else {
        return Ok(());
    };
    let old = (state.x, state.y, state.bottom, state.in_region);
    state.in_region = true;
    state.x = mm(state.margins.left_mm);
    state.y = match kind {
        RegionKind::Header => state.nominal_height - mm(state.margins.top_mm),
        RegionKind::Footer => {
            if state.continuous {
                old.1
            } else {
                mm(state.margins.bottom_mm)
                    + estimate_nodes(
                        &nodes,
                        state.content_width,
                        state.doc.theme.font_size_pt,
                        state.doc.theme.line_height,
                    )?
            }
        }
    };
    if matches!(kind, RegionKind::Footer) {
        state.bottom = mm(state.margins.bottom_mm);
    }
    let draws_before = state.pages.last().map_or(0, |page| page.draws.len());
    layout_nodes(&nodes, state, 1)?;
    if matches!(kind, RegionKind::Header) {
        let page = state
            .pages
            .last_mut()
            .ok_or(RenderError::Invalid("no page"))?;
        page.header_draws = page.draws.len().saturating_sub(draws_before);
    }
    let continuous_footer_y = state.y;
    state.x = old.0;
    state.y = if state.continuous && matches!(kind, RegionKind::Footer) {
        continuous_footer_y
    } else {
        old.1
    };
    state.bottom = old.2;
    state.in_region = old.3;
    Ok(())
}

fn replace_final_header(state: &mut State) -> Result<(), RenderError> {
    let should_replace = state.pages.len() > 1
        && state
            .doc
            .header
            .as_ref()
            .is_some_and(|region| !region.last.is_empty());
    if !should_replace {
        return Ok(());
    }
    let page = state
        .pages
        .last_mut()
        .ok_or(RenderError::Invalid("no page"))?;
    page.draws.drain(..page.header_draws);
    page.header_draws = 0;
    render_region(state, RegionKind::Header, true)
}

fn layout_nodes(nodes: &[Node], state: &mut State, depth: usize) -> Result<(), RenderError> {
    if depth > state.limits.max_depth {
        return Err(RenderError::Limit("nesting depth"));
    }
    for node in nodes {
        if state.pending_page_break && !state.in_region && !matches!(node, Node::PageBreak) {
            state.pending_page_break = false;
            new_page(state)?;
        }
        state.nodes += 1;
        if state.nodes > state.limits.max_nodes {
            return Err(RenderError::Limit("nodes"));
        }
        match node {
            Node::Paragraph { content, style } => paragraph(content, style, state)?,
            Node::Heading {
                content,
                level,
                style,
            } => {
                if !(1..=6).contains(level) {
                    return Err(RenderError::Invalid("heading level"));
                }
                let mut s = style.clone();
                if s.font_size_pt.is_none() {
                    s.font_size_pt = Some(match level {
                        1 => 22.0,
                        2 => 18.0,
                        3 => 15.0,
                        _ => 12.0,
                    })
                }
                s.bold = true;
                paragraph(content, &s, state)?
            }
            Node::Section { children, gap_mm } | Node::Stack { children, gap_mm } => {
                let gap = checked_mm(*gap_mm, "gap")?;
                for (index, child) in children.iter().enumerate() {
                    layout_nodes(std::slice::from_ref(child), state, depth + 1)?;
                    if index + 1 < children.len() {
                        state.y -= gap;
                    }
                }
            }
            Node::Box { children, style } => box_node(children, style, state, depth)?,
            Node::Row { children, gap_mm } => {
                columns(children, &vec![1.0; children.len()], *gap_mm, state, depth)?
            }
            Node::Grid {
                columns: weights,
                children,
                gap_mm,
            } => columns(children, weights, *gap_mm, state, depth)?,
            Node::Table {
                items,
                columns,
                repeat_header,
                empty,
                style,
            } => table(items, columns, *repeat_header, empty, style, state)?,
            Node::Repeat {
                items,
                children,
                gap_mm,
            } => {
                let value = eval(items, state.root, &state.current)?;
                let arr = value
                    .as_array()
                    .ok_or(RenderError::Expression("repeat items must be an array"))?
                    .clone();
                account_repeat(state, arr.len())?;
                let old = state.current.clone();
                for (index, item) in arr.iter().enumerate() {
                    state.current = item.clone();
                    layout_nodes(children, state, depth + 1)?;
                    if index + 1 < arr.len() {
                        state.y -= checked_mm(*gap_mm, "gap")?;
                    }
                }
                state.current = old;
            }
            Node::DataList {
                items,
                header,
                item,
                empty,
                repeat_header,
                gap_mm,
            } => data_list(
                items,
                header,
                item,
                empty,
                *repeat_header,
                *gap_mm,
                state,
                depth,
            )?,
            Node::Conditional {
                condition,
                then,
                otherwise,
            } => {
                if truthy(&eval(condition, state.root, &state.current)?) {
                    layout_nodes(then, state, depth + 1)?
                } else {
                    layout_nodes(otherwise, state, depth + 1)?
                }
            }
            Node::Spacer { height_mm } => {
                let h = checked_mm(*height_mm, "spacer height")?;
                ensure_space(state, h)?;
                state.y -= h
            }
            Node::Divider { width_pt } => {
                if !width_pt.is_finite() || *width_pt <= 0.0 || *width_pt > 10.0 {
                    return Err(RenderError::Invalid("divider width"));
                }
                ensure_space(state, *width_pt + 2.0)?;
                push(
                    state,
                    Draw::Line {
                        x1: state.x,
                        y: state.y,
                        x2: state.x + state.content_width,
                        width: *width_pt,
                        color: state.doc.theme.text_color,
                    },
                )?;
                state.y -= *width_pt + 2.0
            }
            Node::PageBreak => {
                if state.continuous {
                    return Err(RenderError::Unsupported("page breaks on continuous media"));
                }
                // Materialize the next page only if another body node follows.
                // This keeps the natural "break after each repeated order"
                // template from emitting a trailing blank sheet.
                state.pending_page_break = true;
            }
            Node::KeepTogether { children } => {
                let h = estimate_nodes(
                    children,
                    state.content_width,
                    state.doc.theme.font_size_pt,
                    state.doc.theme.line_height,
                )?;
                if h > state.y - state.bottom {
                    if h > state.nominal_height - mm(state.margins.top_mm + state.margins.bottom_mm)
                    {
                        return Err(RenderError::Limit("keep-together block height"));
                    }
                    new_page(state)?;
                }
                let page_count = state.pages.len();
                layout_nodes(children, state, depth + 1)?;
                if state.pages.len() != page_count {
                    return Err(RenderError::Limit("keep-together block height"));
                }
            }
            Node::Image {
                resource,
                width_mm,
                height_mm,
                fit,
            } => image(resource, *width_mm, *height_mm, *fit, state)?,
            Node::ImageValue {
                resource,
                width_mm,
                height_mm,
                fit,
            } => {
                let resource = value_text(&eval(resource, state.root, &state.current)?);
                image(&resource, *width_mm, *height_mm, *fit, state)?
            }
            Node::Qr {
                value,
                size_mm,
                error_correction,
            } => qr(value, *size_mm, *error_correction, state)?,
            Node::Barcode {
                value,
                symbology,
                width_mm,
                height_mm,
                human_readable,
                align,
                padding_mm,
                gap_mm,
            } => barcode(
                value,
                BarcodeLayout {
                    symbology: *symbology,
                    width_mm: *width_mm,
                    height_mm: *height_mm,
                    human_readable: *human_readable,
                    align: *align,
                    padding_mm: *padding_mm,
                    gap_mm: *gap_mm,
                },
                state,
            )?,
        }
    }
    Ok(())
}

fn columns(
    children: &[Node],
    weights: &[f32],
    gap_mm: f32,
    state: &mut State,
    depth: usize,
) -> Result<(), RenderError> {
    if children.is_empty() {
        return Ok(());
    }
    if weights.len() != children.len() {
        return Err(RenderError::Invalid("column count"));
    }
    let gap = checked_mm(gap_mm, "column gap")?;
    let total: f32 = weights.iter().sum();
    let available = state.content_width - gap * (children.len() - 1) as f32;
    if available <= 0.0 {
        return Err(RenderError::Invalid("column width"));
    }
    let old = (state.x, state.y, state.content_width);
    let starting_page = state.pages.len();
    let mut low = state.y;
    for (i, child) in children.iter().enumerate() {
        let prior: f32 = weights[..i].iter().sum();
        state.x = old.0 + available * prior / total + gap * i as f32;
        state.content_width = available * weights[i] / total;
        state.y = old.1;
        layout_nodes(std::slice::from_ref(child), state, depth + 1)?;
        if state.pages.len() != starting_page {
            return Err(RenderError::Unsupported(
                "row and grid children cannot paginate in renderer ABI v1",
            ));
        }
        low = low.min(state.y)
    }
    state.x = old.0;
    state.content_width = old.2;
    state.y = low;
    Ok(())
}

fn box_node(
    children: &[Node],
    style: &BoxStyle,
    state: &mut State,
    depth: usize,
) -> Result<(), RenderError> {
    let padding = checked_mm(style.padding_mm, "box padding")?;
    if state.content_width <= padding * 2.0 {
        return Err(RenderError::Invalid("box content width"));
    }
    let estimated = estimate_nodes(
        children,
        state.content_width - padding * 2.0,
        state.doc.theme.font_size_pt,
        state.doc.theme.line_height,
    )? + padding * 2.0;
    ensure_space(state, estimated)?;
    let page = state.pages.len();
    let draw_index = state.pages[page - 1].draws.len();
    let old = (state.x, state.content_width);
    let top = state.y;
    state.x += padding;
    state.content_width -= padding * 2.0;
    state.y -= padding;
    layout_nodes(children, state, depth + 1)?;
    if state.pages.len() != page {
        return Err(RenderError::Unsupported(
            "box children cannot paginate in renderer ABI v1",
        ));
    }
    state.y -= padding;
    state.x = old.0;
    state.content_width = old.1;
    let height = top - state.y;
    state.pages[page - 1].draws.insert(
        draw_index,
        Draw::Rect {
            x: state.x,
            y: state.y,
            width: state.content_width,
            height,
            fill: style.background,
            stroke: style.border_color,
            stroke_width: style.border_width_pt,
        },
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn data_list(
    items: &Expr,
    header: &[Node],
    item: &[Node],
    empty: &[Node],
    repeat_header: bool,
    gap_mm: f32,
    state: &mut State,
    depth: usize,
) -> Result<(), RenderError> {
    let values = eval(items, state.root, &state.current)?
        .as_array()
        .ok_or(RenderError::Expression("data-list items must be an array"))?
        .clone();
    account_repeat(state, values.len())?;
    if values.is_empty() {
        return layout_nodes(empty, state, depth + 1);
    }
    let gap = checked_mm(gap_mm, "data-list gap")?;
    let old = state.current.clone();
    state.current = values[0].clone();
    let first_item_height = estimate_nodes(
        item,
        state.content_width,
        state.doc.theme.font_size_pt,
        state.doc.theme.line_height,
    )?;
    state.current = old.clone();
    let header_height = estimate_nodes(
        header,
        state.content_width,
        state.doc.theme.font_size_pt,
        state.doc.theme.line_height,
    )?;
    ensure_space(
        state,
        header_height + first_item_height + if values.len() > 1 { gap } else { 0.0 },
    )?;
    render_atomic_nodes(header, state, depth + 1, "data-list header")?;
    for (index, value) in values.iter().enumerate() {
        state.current = value.clone();
        let estimated = estimate_nodes(
            item,
            state.content_width,
            state.doc.theme.font_size_pt,
            state.doc.theme.line_height,
        )? + if index + 1 < values.len() { gap } else { 0.0 };
        if !state.continuous && state.y - estimated < state.bottom {
            new_page(state)?;
            if repeat_header {
                state.current = old.clone();
                render_atomic_nodes(header, state, depth + 1, "data-list header")?;
                state.current = value.clone();
            }
        }
        render_atomic_nodes(item, state, depth + 1, "data-list item")?;
        if index + 1 < values.len() {
            state.y -= gap;
        }
    }
    state.current = old;
    Ok(())
}

fn render_atomic_nodes(
    nodes: &[Node],
    state: &mut State,
    depth: usize,
    label: &'static str,
) -> Result<(), RenderError> {
    if nodes.is_empty() {
        return Ok(());
    }
    let height = estimate_nodes(
        nodes,
        state.content_width,
        state.doc.theme.font_size_pt,
        state.doc.theme.line_height,
    )?;
    ensure_space(state, height)?;
    let page = state.pages.len();
    layout_nodes(nodes, state, depth)?;
    if state.pages.len() != page {
        return Err(RenderError::Unsupported(label));
    }
    Ok(())
}

fn paragraph(content: &[Inline], style: &TextStyle, state: &mut State) -> Result<(), RenderError> {
    let size = style.font_size_pt.unwrap_or(state.doc.theme.font_size_pt);
    if !size.is_finite() || !(4.0..=72.0).contains(&size) {
        return Err(RenderError::Invalid("font size"));
    }
    let mut resolved_style = style.clone();
    if resolved_style.color.is_none() {
        resolved_style.color = Some(state.doc.theme.text_color);
    }
    let runs = resolve_runs(content, state.root, &state.current, &resolved_style, size)?;
    let lines = wrap_runs(runs, state.content_width)?;
    for line in lines {
        let line_h = line
            .iter()
            .map(|run| run.size * state.doc.theme.line_height)
            .fold(size * state.doc.theme.line_height, f32::max);
        ensure_space(state, line_h)?;
        let width = line
            .iter()
            .map(|run| text_width(&run.text, run.size, run.face))
            .sum::<f32>();
        let x = match style.align {
            TextAlign::Left => state.x,
            TextAlign::Center => state.x + (state.content_width - width) / 2.0,
            TextAlign::Right => state.x + state.content_width - width,
        };
        let mut run_x = x;
        for run in line {
            account_text(state, &run.text)?;
            validate_text(&run.text)?;
            let run_width = text_width(&run.text, run.size, run.face);
            push(
                state,
                Draw::Text {
                    x: run_x,
                    y: state.y - run.size,
                    size: run.size,
                    text: run.text,
                    face: run.face,
                    underline: run.underline,
                    color: run.color,
                },
            )?;
            run_x += run_width;
        }
        state.y -= line_h
    }
    Ok(())
}

#[derive(Clone)]
struct StyledRun {
    text: String,
    size: f32,
    face: FontFace,
    underline: bool,
    line_break: bool,
    color: Color,
}

fn resolve_runs(
    content: &[Inline],
    root: &Value,
    current: &Value,
    block: &TextStyle,
    default_size: f32,
) -> Result<Vec<StyledRun>, RenderError> {
    let mut out = Vec::new();
    for inline in content {
        if matches!(inline, Inline::LineBreak) {
            out.push(StyledRun {
                text: String::new(),
                size: default_size,
                face: FontFace::Regular,
                underline: false,
                line_break: true,
                color: block.color.unwrap_or_default(),
            });
            continue;
        }
        let (text, style) = match inline {
            Inline::Text { value, style } => (value.clone(), style),
            Inline::Value {
                value: Expr::PageNumber,
                style,
            } => (PAGE_NUMBER_MARKER.into(), style),
            Inline::Value {
                value: Expr::PageCount,
                style,
            } => (PAGE_COUNT_MARKER.into(), style),
            Inline::Value { value, style } => (value_text(&eval(value, root, current)?), style),
            Inline::LineBreak => unreachable!(),
        };
        let size = style.font_size_pt.unwrap_or(default_size);
        if !size.is_finite() || !(4.0..=72.0).contains(&size) {
            return Err(RenderError::Invalid("inline font size"));
        }
        out.push(StyledRun {
            text,
            size,
            face: match (block.bold || style.bold, block.italic || style.italic) {
                (false, false) => FontFace::Regular,
                (true, false) => FontFace::Bold,
                (false, true) => FontFace::Italic,
                (true, true) => FontFace::BoldItalic,
            },
            underline: block.underline || style.underline,
            line_break: false,
            color: style.color.or(block.color).unwrap_or_default(),
        });
    }
    Ok(out)
}

fn wrap_runs(runs: Vec<StyledRun>, width: f32) -> Result<Vec<Vec<StyledRun>>, RenderError> {
    let mut lines = vec![Vec::new()];
    let mut used = 0.0;
    let mut pending_space: Option<StyledRun> = None;
    for run in runs {
        if run.line_break {
            lines.push(Vec::new());
            used = 0.0;
            pending_space = None;
            continue;
        }

        let mut word = String::new();
        let mut characters = run.text.chars().peekable();
        while let Some(character) = characters.next() {
            if matches!(character, '\r' | '\n') {
                append_wrapped_word(
                    &mut lines,
                    &mut used,
                    width,
                    &mut word,
                    &run,
                    &mut pending_space,
                )?;
                if character == '\r' && characters.peek() == Some(&'\n') {
                    let _ = characters.next();
                }
                lines.push(Vec::new());
                used = 0.0;
                pending_space = None;
            } else if character.is_whitespace() {
                append_wrapped_word(
                    &mut lines,
                    &mut used,
                    width,
                    &mut word,
                    &run,
                    &mut pending_space,
                )?;
                pending_space = Some(run.clone());
            } else {
                word.push(character);
            }
        }
        append_wrapped_word(
            &mut lines,
            &mut used,
            width,
            &mut word,
            &run,
            &mut pending_space,
        )?;
    }
    Ok(lines)
}

fn append_wrapped_word(
    lines: &mut Vec<Vec<StyledRun>>,
    used: &mut f32,
    width: f32,
    word: &mut String,
    style: &StyledRun,
    pending_space: &mut Option<StyledRun>,
) -> Result<(), RenderError> {
    if word.is_empty() {
        return Ok(());
    }

    let word_width = text_width(word, style.size, style.face);
    if word_width > width {
        return Err(RenderError::Limit("unbreakable inline run width"));
    }
    let space_width = if *used > 0.0 {
        pending_space
            .as_ref()
            .map_or(0.0, |space| text_width(" ", space.size, space.face))
    } else {
        0.0
    };
    if *used > 0.0 && *used + space_width + word_width > width {
        lines.push(Vec::new());
        *used = 0.0;
    }

    let line = lines
        .last_mut()
        .ok_or(RenderError::Invalid("inline line"))?;
    if *used > 0.0
        && let Some(space) = pending_space.take()
    {
        append_styled_text(line, &space, " ");
        *used += text_width(" ", space.size, space.face);
    } else {
        *pending_space = None;
    }
    append_styled_text(line, style, word);
    *used += word_width;
    word.clear();
    Ok(())
}

fn append_styled_text(line: &mut Vec<StyledRun>, style: &StyledRun, text: &str) {
    if let Some(previous) = line.last_mut()
        && same_text_style(previous, style)
    {
        previous.text.push_str(text);
        return;
    }
    line.push(StyledRun {
        text: text.to_owned(),
        line_break: false,
        ..style.clone()
    });
}

fn same_text_style(left: &StyledRun, right: &StyledRun) -> bool {
    left.size.to_bits() == right.size.to_bits()
        && left.face == right.face
        && left.underline == right.underline
        && left.color == right.color
}
fn table(
    items: &Expr,
    cols: &[TableColumn],
    repeat_header: bool,
    empty: &[Node],
    style: &TableStyle,
    state: &mut State,
) -> Result<(), RenderError> {
    let value = eval(items, state.root, &state.current)?;
    let rows = value
        .as_array()
        .ok_or(RenderError::Expression("table items must be an array"))?
        .clone();
    account_repeat(state, rows.len())?;
    if rows.is_empty() {
        return layout_nodes(empty, state, 1);
    }
    let weights: Vec<f32> = cols.iter().map(|c| c.width).collect();
    let default_style = TextStyle {
        color: Some(state.doc.theme.text_color),
        ..TextStyle::default()
    };
    let header_style = TextStyle {
        color: style.header_text_color.or(Some(state.doc.theme.text_color)),
        ..TextStyle::default()
    };
    let header: Vec<Vec<StyledRun>> = cols
        .iter()
        .map(|c| {
            resolve_runs(
                &c.header,
                state.root,
                &state.current,
                &header_style,
                state.doc.theme.font_size_pt,
            )
        })
        .collect::<Result<_, _>>()?;
    draw_table_row(&header, cols, &weights, true, style, state)?;
    let old = state.current.clone();
    for row in &rows {
        state.current = row.clone();
        let cells: Vec<Vec<StyledRun>> = cols
            .iter()
            .map(|c| {
                resolve_runs(
                    &c.cell,
                    state.root,
                    row,
                    &default_style,
                    state.doc.theme.font_size_pt,
                )
            })
            .collect::<Result<_, _>>()?;
        let needed = table_row_height(&cells, cols, &weights, style, state);
        if !state.continuous && state.y - needed < state.bottom {
            new_page(state)?;
            if repeat_header {
                draw_table_row(&header, cols, &weights, true, style, state)?
            }
        }
        draw_table_row(&cells, cols, &weights, false, style, state)?;
    }
    state.current = old;
    Ok(())
}
fn table_row_height(
    cells: &[Vec<StyledRun>],
    _cols: &[TableColumn],
    weights: &[f32],
    style: &TableStyle,
    state: &State,
) -> f32 {
    let total: f32 = weights.iter().sum();
    let max = cells
        .iter()
        .zip(weights)
        .map(|(s, w)| {
            let width = state.content_width * *w / total - mm(style.cell_padding_mm) * 2.0;
            wrap_runs(s.clone(), width)
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| {
                            line.iter()
                                .map(|run| run.size * state.doc.theme.line_height)
                                .fold(
                                    state.doc.theme.font_size_pt * state.doc.theme.line_height,
                                    f32::max,
                                )
                        })
                        .sum::<f32>()
                })
                .unwrap_or(f32::INFINITY)
        })
        .fold(0.0_f32, f32::max);
    max + mm(style.cell_padding_mm) * 2.0
}
fn draw_table_row(
    cells: &[Vec<StyledRun>],
    cols: &[TableColumn],
    weights: &[f32],
    header: bool,
    style: &TableStyle,
    state: &mut State,
) -> Result<(), RenderError> {
    let h = table_row_height(cells, cols, weights, style, state);
    ensure_space(state, h)?;
    let padding = mm(style.cell_padding_mm);
    if header && style.header_background.is_some() {
        push(
            state,
            Draw::Rect {
                x: state.x,
                y: state.y - h,
                width: state.content_width,
                height: h,
                fill: style.header_background,
                stroke: None,
                stroke_width: 0.0,
            },
        )?;
    }
    let total: f32 = weights.iter().sum();
    let mut x = state.x;
    for ((cell, col), weight) in cells.iter().zip(cols).zip(weights) {
        let width = state.content_width * *weight / total;
        let lines = wrap_runs(cell.clone(), width - padding * 2.0)?;
        let mut offset_y = 0.0;
        for line in lines {
            let tw = line
                .iter()
                .map(|run| text_width(&run.text, run.size, run.face))
                .sum::<f32>();
            let tx = match col.align {
                TextAlign::Left => x + padding,
                TextAlign::Center => x + (width - tw) / 2.0,
                TextAlign::Right => x + width - tw - padding,
            };
            let line_height = line
                .iter()
                .map(|run| run.size * state.doc.theme.line_height)
                .fold(
                    state.doc.theme.font_size_pt * state.doc.theme.line_height,
                    f32::max,
                );
            let mut run_x = tx;
            for run in line {
                account_text(state, &run.text)?;
                validate_text(&run.text)?;
                let run_width = text_width(&run.text, run.size, run.face);
                push(
                    state,
                    Draw::Text {
                        x: run_x,
                        y: state.y - padding - run.size - offset_y,
                        size: run.size,
                        text: run.text,
                        face: run.face,
                        underline: run.underline,
                        color: run.color,
                    },
                )?;
                run_x += run_width;
            }
            offset_y += line_height;
        }
        x += width
    }
    state.y -= h;
    push(
        state,
        Draw::Line {
            x1: state.x,
            y: state.y,
            x2: state.x + state.content_width,
            width: style.border_width_pt,
            color: style.border_color.unwrap_or(state.doc.theme.text_color),
        },
    )?;
    Ok(())
}

fn qr(expr: &Expr, size_mm: f32, ec: QrCorrection, state: &mut State) -> Result<(), RenderError> {
    let text = value_text(&eval(expr, state.root, &state.current)?);
    account_text(state, &text)?;
    let size = checked_mm(size_mm, "QR size")?;
    if !(mm(10.0)..=mm(100.0)).contains(&size) {
        return Err(RenderError::Invalid("QR size"));
    }
    ensure_space(state, size + 4.0)?;
    let level = match ec {
        QrCorrection::L => EcLevel::L,
        QrCorrection::M => EcLevel::M,
        QrCorrection::Q => EcLevel::Q,
        QrCorrection::H => EcLevel::H,
    };
    let code = QrCode::with_error_correction_level(text.as_bytes(), level)
        .map_err(|_| RenderError::QrTooLarge)?;
    let n = code.width();
    let modules = code
        .into_colors()
        .chunks(n)
        .map(|r| r.iter().map(|c| *c == qrcode::Color::Dark).collect())
        .collect();
    push(
        state,
        Draw::Qr {
            x: state.x,
            y: state.y - size,
            size,
            modules,
        },
    )?;
    state.y -= size + 4.0;
    Ok(())
}
#[derive(Clone, Copy)]
struct BarcodeLayout {
    symbology: BarcodeSymbology,
    width_mm: f32,
    height_mm: f32,
    human_readable: bool,
    align: TextAlign,
    padding_mm: f32,
    gap_mm: f32,
}

fn barcode(expr: &Expr, layout: BarcodeLayout, state: &mut State) -> Result<(), RenderError> {
    let BarcodeLayout {
        symbology: _symbology,
        width_mm,
        height_mm,
        human_readable: human,
        align,
        padding_mm,
        gap_mm,
    } = layout;
    let text = value_text(&eval(expr, state.root, &state.current)?);
    if text.is_empty() || text.len() > 80 || !text.bytes().all(|b| (32..=126).contains(&b)) {
        return Err(RenderError::InvalidBarcode(
            "Code 128 supports 1-80 printable ASCII characters",
        ));
    }
    let width = checked_mm(width_mm, "barcode width")?;
    let height = checked_mm(height_mm, "barcode height")?;
    let padding = checked_mm(padding_mm, "barcode padding")?;
    let gap = checked_mm(gap_mm, "barcode gap")?;
    if width < mm(20.0) || height < mm(8.0) {
        return Err(RenderError::InvalidBarcode(
            "dimensions are below the supported minimum",
        ));
    }
    let bits = code128_bits(&text);
    if width / (bits.len() as f32) < 0.45 {
        return Err(RenderError::InvalidBarcode("module width is too small"));
    }
    let footprint_width = width + padding * 2.0;
    if footprint_width > state.content_width {
        return Err(RenderError::InvalidBarcode(
            "width and padding exceed the available layout width",
        ));
    }
    let line_height = state.doc.theme.font_size_pt * state.doc.theme.line_height;
    let needed = padding * 2.0 + height + if human { gap + line_height } else { 0.0 };
    ensure_space(state, needed)?;
    let footprint_x = match align {
        TextAlign::Left => state.x,
        TextAlign::Center => state.x + (state.content_width - footprint_width) / 2.0,
        TextAlign::Right => state.x + state.content_width - footprint_width,
    };
    let bars_x = footprint_x + padding;
    let bars_y = state.y - padding - height;
    push(
        state,
        Draw::Bars {
            x: bars_x,
            y: bars_y,
            width,
            height,
            bits,
        },
    )?;
    state.y -= padding + height;
    if human {
        state.y -= gap;
        let label_width = text_width(&text, state.doc.theme.font_size_pt, FontFace::Regular);
        if label_width > footprint_width {
            return Err(RenderError::InvalidBarcode(
                "human-readable value exceeds the barcode footprint",
            ));
        }
        account_text(state, &text)?;
        validate_text(&text)?;
        push(
            state,
            Draw::Text {
                x: footprint_x + (footprint_width - label_width) / 2.0,
                y: state.y - state.doc.theme.font_size_pt,
                size: state.doc.theme.font_size_pt,
                text,
                face: FontFace::Regular,
                underline: false,
                color: state.doc.theme.text_color,
            },
        )?;
        state.y -= line_height;
    }
    state.y -= padding;
    Ok(())
}

fn image(
    resource_id: &str,
    width_mm: f32,
    height_mm: f32,
    fit: ImageFit,
    state: &mut State,
) -> Result<(), RenderError> {
    if resource_id.is_empty() || resource_id.len() > 120 {
        return Err(RenderError::Invalid("resource id"));
    }
    let declared = state
        .doc
        .resources
        .get(resource_id)
        .ok_or(RenderError::Invalid("image resource is not declared"))?;
    let Resource::Image { media_type, .. } = declared;
    if media_type != "image/jpeg" {
        return Err(RenderError::Unsupported(
            "renderer ABI v1 supports resolved JPEG images only",
        ));
    }
    let bytes = state
        .resolved
        .images
        .get(resource_id)
        .ok_or(RenderError::Unsupported(
            "image bytes must be supplied through ResolvedResources",
        ))?;
    let (pixel_width, pixel_height) = jpeg_dimensions(bytes)?;
    let box_width = checked_mm(width_mm, "image width")?;
    let box_height = checked_mm(height_mm, "image height")?;
    if box_width == 0.0 || box_height == 0.0 {
        return Err(RenderError::Invalid("image dimensions"));
    }
    if matches!(fit, ImageFit::Cover) {
        return Err(RenderError::Unsupported(
            "image cover cropping is not available in renderer ABI v1",
        ));
    }
    let (width, height) = if matches!(fit, ImageFit::Fill) {
        (box_width, box_height)
    } else {
        let scale = (box_width / f32::from(pixel_width)).min(box_height / f32::from(pixel_height));
        let scale = if matches!(fit, ImageFit::ScaleDown) {
            scale.min(1.0)
        } else {
            scale
        };
        (
            f32::from(pixel_width) * scale,
            f32::from(pixel_height) * scale,
        )
    };
    ensure_space(state, box_height)?;
    push(
        state,
        Draw::Jpeg {
            x: state.x,
            y: state.y - height,
            width,
            height,
            pixel_width,
            pixel_height,
            resource_id: resource_id.to_owned(),
        },
    )?;
    state.y -= box_height;
    Ok(())
}

fn jpeg_dimensions(bytes: &[u8]) -> Result<(u16, u16), RenderError> {
    if bytes.len() < 4 || !bytes.starts_with(&[0xFF, 0xD8]) || !bytes.ends_with(&[0xFF, 0xD9]) {
        return Err(RenderError::Invalid("malformed JPEG"));
    }
    let mut offset = 2usize;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xFF {
            return Err(RenderError::Invalid("malformed JPEG marker"));
        }
        while offset < bytes.len() && bytes[offset] == 0xFF {
            offset += 1;
        }
        let marker = *bytes
            .get(offset)
            .ok_or(RenderError::Invalid("malformed JPEG marker"))?;
        offset += 1;
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        let length = bytes
            .get(offset..offset + 2)
            .map(|value| usize::from(u16::from_be_bytes([value[0], value[1]])))
            .ok_or(RenderError::Invalid("malformed JPEG segment"))?;
        if length < 2 || offset + length > bytes.len() {
            return Err(RenderError::Invalid("malformed JPEG segment"));
        }
        if matches!(marker, 0xC0..=0xC2) {
            let segment = &bytes[offset + 2..offset + length];
            if segment.len() < 6 || segment[0] != 8 || segment[5] != 3 {
                return Err(RenderError::Unsupported(
                    "JPEG must use 8-bit baseline/progressive RGB samples",
                ));
            }
            let height = u16::from_be_bytes([segment[1], segment[2]]);
            let width = u16::from_be_bytes([segment[3], segment[4]]);
            if width == 0 || height == 0 {
                return Err(RenderError::Invalid("JPEG has zero dimensions"));
            }
            if u32::from(width) * u32::from(height) > 50_000_000 {
                return Err(RenderError::Limit("JPEG pixels"));
            }
            return Ok((width, height));
        }
        offset += length;
    }
    Err(RenderError::Invalid("JPEG dimensions were not found"))
}

fn eval(expr: &Expr, root: &Value, current: &Value) -> Result<Value, RenderError> {
    match expr {
        Expr::Literal { value } => Ok(value.clone()),
        Expr::Path { path } => walk(root, path),
        Expr::CurrentPath { path } => walk(current, path),
        Expr::Coalesce { values } => {
            for e in values {
                let v = match eval(e, root, current) {
                    Ok(value) => value,
                    Err(RenderError::MissingPath(_)) => continue,
                    Err(error) => return Err(error),
                };
                if !v.is_null() {
                    return Ok(v);
                }
            }
            Ok(Value::Null)
        }
        Expr::Concat { values } => Ok(Value::String(
            values
                .iter()
                .map(|e| eval(e, root, current).map(|v| value_text(&v)))
                .collect::<Result<Vec<_>, _>>()?
                .join(""),
        )),
        Expr::Compare {
            operator,
            left,
            right,
        } => {
            let l = eval(left, root, current)?;
            let r = eval(right, root, current)?;
            Ok(Value::Bool(compare(&l, &r, *operator)?))
        }
        Expr::Boolean { operator, values } => {
            if values.is_empty() || values.len() > 100 {
                return Err(RenderError::Limit("boolean operands"));
            }
            let vals = values
                .iter()
                .map(|e| eval(e, root, current).map(|v| truthy(&v)))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Bool(match operator {
                BooleanOperator::And => vals.iter().all(|v| *v),
                BooleanOperator::Or => vals.iter().any(|v| *v),
            }))
        }
        Expr::Not { value } => Ok(Value::Bool(!truthy(&eval(value, root, current)?))),
        Expr::Exists { value } => match eval(value, root, current) {
            Ok(value) => Ok(Value::Bool(!value.is_null())),
            Err(RenderError::MissingPath(_)) => Ok(Value::Bool(false)),
            Err(error) => Err(error),
        },
        Expr::Contains { collection, value } => {
            let collection = eval(collection, root, current)?;
            let value = eval(value, root, current)?;
            let contains = match collection {
                Value::Array(values) => values.iter().any(|candidate| candidate == &value),
                Value::String(text) => value
                    .as_str()
                    .map(|candidate| text.contains(candidate))
                    .ok_or(RenderError::Expression(
                        "string contains requires a string value",
                    ))?,
                _ => {
                    return Err(RenderError::Expression(
                        "contains requires an array or string collection",
                    ));
                }
            };
            Ok(Value::Bool(contains))
        }
        Expr::PageNumber | Expr::PageCount => Err(RenderError::Unsupported(
            "page context expressions are supported only as direct inline values",
        )),
        Expr::Arithmetic {
            operator,
            left,
            right,
        } => {
            let l = number(&eval(left, root, current)?)?;
            let r = number(&eval(right, root, current)?)?;
            let n = match operator {
                ArithmeticOperator::Add => l + r,
                ArithmeticOperator::Subtract => l - r,
                ArithmeticOperator::Multiply => l * r,
                ArithmeticOperator::Divide => {
                    if r == 0.0 {
                        return Err(RenderError::Expression("division by zero"));
                    }
                    l / r
                }
            };
            serde_json::Number::from_f64(n)
                .map(Value::Number)
                .ok_or(RenderError::Expression("non-finite arithmetic"))
        }
        Expr::FormatNumber { value, decimals } => {
            if *decimals > 12 {
                return Err(RenderError::Invalid("number decimals"));
            }
            Ok(Value::String(format!(
                "{:.*}",
                *decimals as usize,
                number(&eval(value, root, current)?)?
            )))
        }
        Expr::FormatMoney {
            amount,
            currency,
            decimals,
        } => {
            if *decimals > 6 {
                return Err(RenderError::Invalid("money decimals"));
            }
            let n = number(&eval(amount, root, current)?)?;
            let c = value_text(&eval(currency, root, current)?);
            if c.len() != 3 || !c.bytes().all(|b| b.is_ascii_uppercase()) {
                return Err(RenderError::Expression(
                    "currency must be a three-letter uppercase code",
                ));
            }
            Ok(Value::String(format!("{} {:.*}", c, *decimals as usize, n)))
        }
        Expr::FormatDate { value, format } => {
            let raw = value_text(&eval(value, root, current)?);
            let date =
                raw.get(..10)
                    .filter(|value| {
                        value.as_bytes().get(4) == Some(&b'-')
                            && value.as_bytes().get(7) == Some(&b'-')
                            && value.bytes().enumerate().all(|(index, byte)| {
                                index == 4 || index == 7 || byte.is_ascii_digit()
                            })
                    })
                    .ok_or(RenderError::Expression("date must start with YYYY-MM-DD"))?;
            let year = &date[0..4];
            let month = &date[5..7];
            let day = &date[8..10];
            Ok(Value::String(match format {
                DateFormat::IsoDate => date.to_owned(),
                DateFormat::DayMonthYear => format!("{day}/{month}/{year}"),
                DateFormat::MonthDayYear => format!("{month}/{day}/{year}"),
            }))
        }
        Expr::FormatString { value, operation } => {
            let raw = value_text(&eval(value, root, current)?);
            Ok(Value::String(match operation {
                StringOperation::Trim => raw.trim().to_owned(),
                StringOperation::UppercaseAscii => raw.to_ascii_uppercase(),
                StringOperation::LowercaseAscii => raw.to_ascii_lowercase(),
            }))
        }
    }
}
fn walk(base: &Value, path: &[String]) -> Result<Value, RenderError> {
    if path.len() > 64 {
        return Err(RenderError::Limit("expression path depth"));
    }
    let mut v = base;
    for p in path {
        v = match v {
            Value::Object(m) => m.get(p),
            Value::Array(a) => p.parse::<usize>().ok().and_then(|i| a.get(i)),
            _ => None,
        }
        .ok_or_else(|| RenderError::MissingPath(path.join(".")))?;
    }
    Ok(v.clone())
}
fn compare(l: &Value, r: &Value, op: CompareOperator) -> Result<bool, RenderError> {
    match op {
        CompareOperator::Equal => Ok(l == r),
        CompareOperator::NotEqual => Ok(l != r),
        _ => {
            let (a, b) = if l.is_number() && r.is_number() {
                (number(l)?, number(r)?)
            } else {
                return Err(RenderError::Expression(
                    "ordered comparison requires numbers",
                ));
            };
            Ok(match op {
                CompareOperator::Less => a < b,
                CompareOperator::LessOrEqual => a <= b,
                CompareOperator::Greater => a > b,
                CompareOperator::GreaterOrEqual => a >= b,
                _ => false,
            })
        }
    }
}
fn number(v: &Value) -> Result<f64, RenderError> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or(RenderError::Expression("number required"))
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
    }
}
fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}
fn ensure_space(state: &mut State, h: f32) -> Result<(), RenderError> {
    if !h.is_finite() || h < 0.0 {
        return Err(RenderError::Invalid("element height"));
    }
    if state.continuous {
        return Ok(());
    }
    if state.y - h < state.bottom {
        if state.in_region {
            return Err(RenderError::Limit("header/footer overflow"));
        }
        new_page(state)?;
    }
    if state.y - h < state.bottom {
        return Err(RenderError::Limit("element exceeds page"));
    }
    Ok(())
}
fn new_page(state: &mut State) -> Result<(), RenderError> {
    if state.continuous {
        return Err(RenderError::Unsupported("page breaks on continuous media"));
    }
    if state.pages.len() >= state.limits.max_pages {
        return Err(RenderError::Limit("pages"));
    }
    // Close the current page with its ordinary repeated footer before creating
    // the next page. The final page is closed by `render` with its last/default
    // footer, so a footer is emitted exactly once per page.
    render_region(state, RegionKind::Footer, false)?;
    state.pages.push(PageDraw {
        width: state.width,
        height: state.nominal_height,
        draws: vec![],
        header_draws: 0,
    });
    state.x = mm(state.margins.left_mm);
    state.y = state.nominal_height - mm(state.margins.top_mm);
    reserve_regions(state)?;
    render_region(state, RegionKind::Header, false)
}
fn push(state: &mut State, draw: Draw) -> Result<(), RenderError> {
    let estimate = match &draw {
        Draw::Text { text, .. } => text.len().saturating_mul(4).saturating_add(256),
        Draw::Line { .. } | Draw::Rect { .. } | Draw::Jpeg { .. } => 256,
        Draw::Qr { modules, .. } => modules
            .iter()
            .map(|row| row.iter().filter(|module| **module).count())
            .sum::<usize>()
            .saturating_mul(64)
            .saturating_add(256),
        Draw::Bars { bits, .. } => bits
            .iter()
            .filter(|module| **module)
            .count()
            .saturating_mul(64)
            .saturating_add(256),
    };
    state.estimated_pdf_bytes = state
        .estimated_pdf_bytes
        .checked_add(estimate)
        .ok_or(RenderError::Limit("output bytes"))?;
    if state.estimated_pdf_bytes > state.limits.max_output_bytes {
        return Err(RenderError::Limit("output bytes"));
    }
    state
        .pages
        .last_mut()
        .ok_or(RenderError::Invalid("no page"))?
        .draws
        .push(draw);
    Ok(())
}
const fn account_repeat(state: &mut State, n: usize) -> Result<(), RenderError> {
    state.repeats = state.repeats.saturating_add(n);
    if state.repeats > state.limits.max_repeat_items {
        Err(RenderError::Limit("repeat items"))
    } else {
        Ok(())
    }
}
const fn account_text(state: &mut State, s: &str) -> Result<(), RenderError> {
    state.text_bytes = state.text_bytes.saturating_add(s.len());
    if state.text_bytes > state.limits.max_text_bytes {
        Err(RenderError::Limit("text bytes"))
    } else {
        Ok(())
    }
}
fn validate_text(s: &str) -> Result<(), RenderError> {
    for c in s.chars() {
        if matches!(c, '\u{e000}' | '\u{e001}') {
            continue;
        }
        if c != '\n' && c != '\r' && c != '\t' && (encode_win_ansi(c).is_none() || c.is_control()) {
            return Err(RenderError::UnsupportedCharacter { code: c as u32 });
        }
    }
    Ok(())
}
fn text_width(s: &str, size: f32, face: FontFace) -> f32 {
    let bold = matches!(face, FontFace::Bold | FontFace::BoldItalic);
    display_width_text(s)
        .chars()
        .map(|character| f32::from(helvetica_width(character, bold)))
        .sum::<f32>()
        * size
        / 1_000.0
}

fn helvetica_width(character: char, bold: bool) -> u16 {
    const REGULAR_ASCII: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556,
        556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722,
        722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722,
        667, 944, 667, 667, 611, 333, 278, 333, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556,
        556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500,
        500, 334, 260, 334, 584,
    ];
    const BOLD_ASCII: [u16; 95] = [
        278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556,
        556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722,
        722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722,
        667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611,
        611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556,
        500, 389, 280, 389, 584,
    ];

    if character.is_ascii() && (' '..='~').contains(&character) {
        let index = character as usize - ' ' as usize;
        return if bold {
            BOLD_ASCII[index]
        } else {
            REGULAR_ASCII[index]
        };
    }

    if let Some(width) = helvetica_extended_width(character, bold) {
        return width;
    }

    let base = match character {
        '\u{00a0}' => ' ',
        '\u{00c0}'..='\u{00c5}' => 'A',
        '\u{00c7}' => 'C',
        '\u{00c8}'..='\u{00cb}' => 'E',
        '\u{00cc}'..='\u{00cf}' => 'I',
        '\u{00d0}' => 'D',
        '\u{00d1}' => 'N',
        '\u{00d2}'..='\u{00d6}' | '\u{00d8}' => 'O',
        '\u{00d9}'..='\u{00dc}' => 'U',
        '\u{00dd}' | '\u{0178}' => 'Y',
        '\u{0161}' | '\u{017e}' => 's',
        '\u{00e0}'..='\u{00e5}' => 'a',
        '\u{00e7}' => 'c',
        '\u{00e8}'..='\u{00eb}' => 'e',
        '\u{00ec}'..='\u{00ef}' => 'i',
        '\u{00f1}' => 'n',
        '\u{00f2}'..='\u{00f6}' => 'o',
        '\u{00f9}'..='\u{00fc}' => 'u',
        '\u{00fd}' | '\u{00ff}' => 'y',
        '\u{0160}' | '\u{017d}' => 'S',
        '\u{0192}' => 'f',
        _ => return 556,
    };
    let index = base as usize - ' ' as usize;
    if bold {
        BOLD_ASCII[index]
    } else {
        REGULAR_ASCII[index]
    }
}

const fn helvetica_extended_width(character: char, bold: bool) -> Option<u16> {
    let width = match character {
        '\u{00a6}' => {
            if bold {
                280
            } else {
                260
            }
        }
        '\u{00b5}' | '\u{00f0}' | '\u{00fe}' => {
            if bold {
                611
            } else {
                556
            }
        }
        '\u{00b6}' => {
            if bold {
                556
            } else {
                537
            }
        }
        '\u{2018}' | '\u{2019}' | '\u{201a}' => {
            if bold {
                278
            } else {
                222
            }
        }
        '\u{201c}' | '\u{201d}' | '\u{201e}' => {
            if bold {
                500
            } else {
                333
            }
        }
        '\u{00a0}' | '\u{00b7}' => 278,
        '\u{00a1}'
        | '\u{00a8}'
        | '\u{00ad}'
        | '\u{00af}'
        | '\u{00b2}'..='\u{00b4}'
        | '\u{00b8}'
        | '\u{00b9}'
        | '\u{02c6}'
        | '\u{02dc}'
        | '\u{2039}'
        | '\u{203a}' => 333,
        '\u{2022}' => 350,
        '\u{00ba}' => 365,
        '\u{00aa}' => 370,
        '\u{00b0}' => 400,
        '\u{017e}' => 500,
        '\u{00a2}'..='\u{00a5}'
        | '\u{00a7}'
        | '\u{00ab}'
        | '\u{00bb}'
        | '\u{0192}'
        | '\u{2013}'
        | '\u{2020}'
        | '\u{2021}'
        | '\u{20ac}' => 556,
        '\u{00ac}' | '\u{00b1}' | '\u{00d7}' | '\u{00f7}' => 584,
        '\u{00bf}' | '\u{00df}' | '\u{00f8}' | '\u{017d}' => 611,
        '\u{00de}' => 667,
        '\u{00a9}' | '\u{00ae}' => 737,
        '\u{00bc}'..='\u{00be}' => 834,
        '\u{00e6}' => 889,
        '\u{0153}' => 944,
        '\u{00c6}' | '\u{0152}' | '\u{2014}' | '\u{2026}' | '\u{2030}' | '\u{2122}' => 1_000,
        _ => return None,
    };
    Some(width)
}
fn display_width_text(value: &str) -> String {
    value
        .replace(PAGE_NUMBER_MARKER, "0000")
        .replace(PAGE_COUNT_MARKER, "0000")
}
fn checked_mm(v: f32, label: &'static str) -> Result<f32, RenderError> {
    if !v.is_finite() || !(0.0..=2_000.0).contains(&v) {
        Err(RenderError::Invalid(label))
    } else {
        Ok(mm(v))
    }
}
fn mm(v: f32) -> f32 {
    v * 72.0 / 25.4
}
fn estimate_nodes(nodes: &[Node], width: f32, size: f32, line: f32) -> Result<f32, RenderError> {
    let mut h = 0.0;
    for n in nodes {
        h += match n {
            Node::Paragraph { content, .. } | Node::Heading { content, .. } => {
                let literal = content
                    .iter()
                    .map(|i| match i {
                        Inline::Text { value, .. } => value.len(),
                        _ => 8,
                    })
                    .sum::<usize>();
                let chars = ((width / (size * 0.52)).floor() as usize).max(1);
                literal.div_ceil(chars) as f32 * size * line
            }
            Node::Spacer { height_mm } => checked_mm(*height_mm, "spacer")?,
            Node::Divider { .. } => 3.0,
            Node::Qr { size_mm, .. } => checked_mm(*size_mm, "QR")?,
            Node::Barcode {
                height_mm,
                human_readable,
                padding_mm,
                gap_mm,
                ..
            } => {
                checked_mm(*height_mm, "barcode")?
                    + checked_mm(*padding_mm, "barcode padding")? * 2.0
                    + if *human_readable {
                        checked_mm(*gap_mm, "barcode gap")? + size * line
                    } else {
                        0.0
                    }
            }
            Node::Image { height_mm, .. } | Node::ImageValue { height_mm, .. } => {
                checked_mm(*height_mm, "image height")?
            }
            Node::Section { children, gap_mm } | Node::Stack { children, gap_mm } => {
                estimate_nodes(children, width, size, line)?
                    + checked_mm(*gap_mm, "gap")? * children.len().saturating_sub(1) as f32
            }
            Node::KeepTogether { children } => estimate_nodes(children, width, size, line)?,
            Node::Row { children, gap_mm } => {
                let gap = checked_mm(*gap_mm, "column gap")?;
                let child_width = (width - gap * children.len().saturating_sub(1) as f32)
                    / children.len().max(1) as f32;
                children
                    .iter()
                    .map(|child| {
                        estimate_nodes(std::slice::from_ref(child), child_width, size, line)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .fold(0.0_f32, f32::max)
            }
            Node::Grid {
                columns,
                children,
                gap_mm,
            } => {
                let gap = checked_mm(*gap_mm, "column gap")?;
                let available = width - gap * children.len().saturating_sub(1) as f32;
                let total: f32 = columns.iter().sum();
                children
                    .iter()
                    .zip(columns)
                    .map(|(child, weight)| {
                        estimate_nodes(
                            std::slice::from_ref(child),
                            available * *weight / total,
                            size,
                            line,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .fold(0.0_f32, f32::max)
            }
            Node::Box { children, style } => {
                let padding = checked_mm(style.padding_mm, "box padding")?;
                estimate_nodes(children, width - padding * 2.0, size, line)? + padding * 2.0
            }
            Node::DataList {
                header,
                item,
                gap_mm,
                ..
            } => {
                estimate_nodes(header, width, size, line)?
                    + estimate_nodes(item, width, size, line)?
                    + checked_mm(*gap_mm, "data-list gap")?
            }
            _ => size * line,
        };
    }
    Ok(h)
}
fn translate_y(draw: &mut Draw, delta: f32) {
    match draw {
        Draw::Text { y, .. }
        | Draw::Line { y, .. }
        | Draw::Rect { y, .. }
        | Draw::Qr { y, .. }
        | Draw::Bars { y, .. }
        | Draw::Jpeg { y, .. } => *y += delta,
    }
}

// Code 128-B patterns, including start-B/checksum/stop. Each digit is a module width.
const CODE128: &[&str] = &[
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];
fn code128_bits(s: &str) -> Vec<bool> {
    let mut codes = vec![104usize];
    codes.extend(s.bytes().map(|b| (b - 32) as usize));
    let sum = codes
        .iter()
        .enumerate()
        .skip(1)
        .fold(104usize, |a, (i, c)| a + i * c)
        % 103;
    codes.push(sum);
    codes.push(106);
    // ISO/IEC 15417 requires quiet areas on both sides. Ten modules is the
    // minimum for Code 128 and is part of the encoded width, never caller data.
    let mut bits = vec![false; 10];
    for code in codes {
        let mut bar = true;
        for d in CODE128[code].bytes() {
            for _ in 0..(d - b'0') {
                bits.push(bar)
            }
            bar = !bar
        }
    }
    bits.extend(std::iter::repeat_n(false, 10));
    bits
}

fn pdf_escape(s: &str) -> String {
    let mut output = String::new();
    for character in s.chars().filter(|character| *character != '\r') {
        let Some(byte) = encode_win_ansi(character) else {
            continue;
        };
        match byte {
            b'\\' => output.push_str("\\\\"),
            b'(' => output.push_str("\\("),
            b')' => output.push_str("\\)"),
            32..=126 => output.push(char::from(byte)),
            _ => {
                let _ = write!(output, "\\{byte:03o}");
            }
        }
    }
    output
}

fn pdf_color(color: Color) -> String {
    format!(
        "{:.4} {:.4} {:.4}",
        f32::from(color.red) / 255.0,
        f32::from(color.green) / 255.0,
        f32::from(color.blue) / 255.0
    )
}

fn encode_win_ansi(character: char) -> Option<u8> {
    match u32::from(character) {
        0x20..=0x7e | 0xa0..=0xff => u8::try_from(u32::from(character)).ok(),
        0x20ac => Some(0x80),
        0x201a => Some(0x82),
        0x0192 => Some(0x83),
        0x201e => Some(0x84),
        0x2026 => Some(0x85),
        0x2020 => Some(0x86),
        0x2021 => Some(0x87),
        0x02c6 => Some(0x88),
        0x2030 => Some(0x89),
        0x0160 => Some(0x8a),
        0x2039 => Some(0x8b),
        0x0152 => Some(0x8c),
        0x017d => Some(0x8e),
        0x2018 => Some(0x91),
        0x2019 => Some(0x92),
        0x201c => Some(0x93),
        0x201d => Some(0x94),
        0x2022 => Some(0x95),
        0x2013 => Some(0x96),
        0x2014 => Some(0x97),
        0x02dc => Some(0x98),
        0x2122 => Some(0x99),
        0x0161 => Some(0x9a),
        0x203a => Some(0x9b),
        0x0153 => Some(0x9c),
        0x017e => Some(0x9e),
        0x0178 => Some(0x9f),
        _ => None,
    }
}

fn write_pdf(
    pages: &[PageDraw],
    resolved: &ResolvedResources,
    max_output_bytes: usize,
) -> Result<Vec<u8>, RenderError> {
    let page_count = pages.len();
    let font_id = 3 + page_count * 2;
    let first_image_id = font_id + 4;
    let mut used_images = BTreeMap::<String, (u16, u16)>::new();
    for page in pages {
        for draw in &page.draws {
            if let Draw::Jpeg {
                resource_id,
                pixel_width,
                pixel_height,
                ..
            } = draw
            {
                used_images.insert(resource_id.clone(), (*pixel_width, *pixel_height));
            }
        }
    }
    let image_ids = used_images
        .keys()
        .enumerate()
        .map(|(index, resource_id)| (resource_id.clone(), first_image_id + index))
        .collect::<BTreeMap<_, _>>();
    let mut objects = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()];
    let kids = (0..page_count)
        .map(|index| format!("{} 0 R", 3 + index * 2))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>").into_bytes());
    for (page_index, page) in pages.iter().enumerate() {
        let page_id = 3 + page_index * 2;
        let content_id = page_id + 1;
        let xobjects = page
            .draws
            .iter()
            .enumerate()
            .filter_map(|(draw_index, draw)| {
                let Draw::Jpeg { resource_id, .. } = draw else {
                    return None;
                };
                image_ids
                    .get(resource_id)
                    .map(|id| format!("/Im{draw_index} {id} 0 R"))
            })
            .collect::<Vec<_>>()
            .join(" ");
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources << /Font << /F1 {font_id} 0 R /F2 {} 0 R /F3 {} 0 R /F4 {} 0 R >> /XObject << {xobjects} >> >> /Contents {content_id} 0 R >>",page.width,page.height,font_id+1,font_id+2,font_id+3).into_bytes());
        let mut stream = String::new();
        for (draw_index, draw) in page.draws.iter().enumerate() {
            match draw {
                Draw::Text {
                    x,
                    y,
                    size,
                    text,
                    face,
                    underline,
                    color,
                } => {
                    let rendered_text = text
                        .replace(PAGE_NUMBER_MARKER, &(page_index + 1).to_string())
                        .replace(PAGE_COUNT_MARKER, &page_count.to_string());
                    let font = match face {
                        FontFace::Regular => "F1",
                        FontFace::Bold => "F2",
                        FontFace::Italic => "F3",
                        FontFace::BoldItalic => "F4",
                    };
                    let _ = writeln!(
                        stream,
                        "{} rg BT /{font} {size:.2} Tf {x:.2} {y:.2} Td ({}) Tj ET",
                        pdf_color(*color),
                        pdf_escape(&rendered_text)
                    );
                    if *underline {
                        let x2 = *x + text_width(&rendered_text, *size, *face);
                        let line_y = *y - 1.2;
                        let _ = writeln!(
                            stream,
                            "{} RG 0.5 w {x:.2} {line_y:.2} m {x2:.2} {line_y:.2} l S",
                            pdf_color(*color)
                        );
                    }
                }
                Draw::Line {
                    x1,
                    y,
                    x2,
                    width,
                    color,
                } => {
                    let _ = writeln!(
                        stream,
                        "{} RG {width:.2} w {x1:.2} {y:.2} m {x2:.2} {y:.2} l S",
                        pdf_color(*color)
                    );
                }
                Draw::Rect {
                    x,
                    y,
                    width,
                    height,
                    fill,
                    stroke,
                    stroke_width,
                } => {
                    if let Some(color) = fill {
                        let _ = writeln!(
                            stream,
                            "{} rg {x:.2} {y:.2} {width:.2} {height:.2} re f",
                            pdf_color(*color)
                        );
                    }
                    if let Some(color) = stroke {
                        if *stroke_width > 0.0 {
                            let _ = writeln!(
                                stream,
                                "{} RG {stroke_width:.2} w {x:.2} {y:.2} {width:.2} {height:.2} re S",
                                pdf_color(*color)
                            );
                        }
                    }
                }
                Draw::Qr {
                    x,
                    y,
                    size,
                    modules,
                } => {
                    stream.push_str("0 0 0 rg\n");
                    let count = modules.len() as f32;
                    let module = *size / (count + 8.0);
                    for (row, values) in modules.iter().enumerate() {
                        for (col, dark) in values.iter().enumerate() {
                            if *dark {
                                let px = *x + (col as f32 + 4.0) * module;
                                let py = *y + (count - row as f32 + 3.0) * module;
                                let _ = writeln!(
                                    stream,
                                    "{px:.3} {py:.3} {module:.3} {module:.3} re f"
                                );
                            }
                        }
                    }
                }
                Draw::Bars {
                    x,
                    y,
                    width,
                    height,
                    bits,
                } => {
                    stream.push_str("0 0 0 rg\n");
                    let module = *width / bits.len() as f32;
                    for (index, dark) in bits.iter().enumerate() {
                        if *dark {
                            let px = *x + index as f32 * module;
                            let _ = writeln!(stream, "{px:.3} {y:.3} {module:.3} {height:.3} re f");
                        }
                    }
                }
                Draw::Jpeg {
                    x,
                    y,
                    width,
                    height,
                    ..
                } => {
                    let _ = writeln!(
                        stream,
                        "q {width:.3} 0 0 {height:.3} {x:.3} {y:.3} cm /Im{draw_index} Do Q"
                    );
                }
            }
        }
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{}endstream",
                stream.len(),
                stream
            )
            .into_bytes(),
        );
    }
    for name in [
        "Helvetica",
        "Helvetica-Bold",
        "Helvetica-Oblique",
        "Helvetica-BoldOblique",
    ] {
        objects.push(
            format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>"
            )
            .into_bytes(),
        );
    }
    for (resource_id, (pixel_width, pixel_height)) in &used_images {
        let bytes = resolved
            .images
            .get(resource_id)
            .ok_or(RenderError::Invalid("resolved resource set mismatch"))?;
        let mut object=format!("<< /Type /XObject /Subtype /Image /Width {pixel_width} /Height {pixel_height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",bytes.len()).into_bytes();
        object.extend_from_slice(bytes);
        object.extend_from_slice(b"\nendstream");
        objects.push(object);
    }
    let object_bytes = objects.iter().try_fold(0_usize, |total, object| {
        total
            .checked_add(object.len())
            .ok_or(RenderError::Limit("output bytes"))
    })?;
    if object_bytes > max_output_bytes {
        return Err(RenderError::Limit("output bytes"));
    }
    let mut pdf = b"%PDF-1.4\n% PrintPacket\n".to_vec();
    let mut offsets = vec![0usize];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        append_pdf(
            &mut pdf,
            format!("{} 0 obj\n", index + 1).as_bytes(),
            max_output_bytes,
        )?;
        append_pdf(&mut pdf, object, max_output_bytes)?;
        append_pdf(&mut pdf, b"\nendobj\n", max_output_bytes)?;
    }
    let xref = pdf.len();
    append_pdf(
        &mut pdf,
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        max_output_bytes,
    )?;
    for offset in offsets.iter().skip(1) {
        append_pdf(
            &mut pdf,
            format!("{offset:010} 00000 n \n").as_bytes(),
            max_output_bytes,
        )?;
    }
    append_pdf(
        &mut pdf,
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
        max_output_bytes,
    )?;
    Ok(pdf)
}

fn append_pdf(output: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<(), RenderError> {
    if output
        .len()
        .checked_add(bytes.len())
        .is_none_or(|length| length > limit)
    {
        return Err(RenderError::Limit("output bytes"));
    }
    output.extend_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default, clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;
    fn text(s: &str) -> Node {
        Node::Paragraph {
            content: vec![Inline::Text {
                value: s.into(),
                style: TextStyle::default(),
            }],
            style: TextStyle::default(),
        }
    }
    fn document(body: Vec<Node>) -> PrintPacketV1 {
        PrintPacketV1 {
            format: PRINT_PACKET_DOCUMENT_FORMAT.into(),
            media: Media::Paged {
                size: PageSize::A4,
                orientation: Orientation::Portrait,
                margins: default_margins(),
            },
            theme: Theme::default(),
            resources: BTreeMap::new(),
            header: None,
            body,
            footer: None,
        }
    }
    fn test_state<'a>(
        doc: &'a PrintPacketV1,
        root: &'a Value,
        resolved: &'a ResolvedResources,
    ) -> State<'a> {
        let limits = RenderLimits::default();
        let (width, height, margins, continuous) = media_geometry(&doc.media, limits).unwrap();
        State {
            doc,
            limits,
            root,
            current: root.clone(),
            resolved,
            pages: vec![PageDraw {
                width,
                height,
                draws: vec![],
                header_draws: 0,
            }],
            width,
            nominal_height: height,
            margins,
            x: mm(margins.left_mm),
            y: height - mm(margins.top_mm),
            content_width: width - mm(margins.left_mm + margins.right_mm),
            bottom: mm(margins.bottom_mm),
            nodes: 0,
            repeats: 0,
            text_bytes: 0,
            continuous,
            in_region: false,
            pending_page_break: false,
            estimated_pdf_bytes: 4_096,
        }
    }
    fn path(name: &str) -> Expr {
        Expr::CurrentPath {
            path: vec![name.into()],
        }
    }

    fn decode_code128_b(bits: &[bool]) -> Result<(Vec<usize>, String), &'static str> {
        if bits.len() < 20
            || bits[..10].iter().any(|module| *module)
            || bits[bits.len() - 10..].iter().any(|module| *module)
        {
            return Err("missing quiet zone");
        }
        let payload = &bits[10..bits.len() - 10];
        let mut position = 0;
        let mut codes = Vec::new();
        while position < payload.len() {
            let width = if payload.len() - position == 13 {
                13
            } else {
                11
            };
            let chunk = payload
                .get(position..position + width)
                .ok_or("truncated symbol")?;
            let code = CODE128
                .iter()
                .enumerate()
                .find_map(|(code, pattern)| {
                    let mut expected = Vec::new();
                    let mut bar = true;
                    for digit in pattern.bytes() {
                        expected.extend(std::iter::repeat_n(bar, usize::from(digit - b'0')));
                        bar = !bar;
                    }
                    (expected == chunk).then_some(code)
                })
                .ok_or("unknown codeword")?;
            codes.push(code);
            position += width;
        }
        if codes.first() != Some(&104) || codes.last() != Some(&106) || codes.len() < 3 {
            return Err("invalid start/stop");
        }
        let checksum = codes[..codes.len() - 2]
            .iter()
            .enumerate()
            .skip(1)
            .fold(104_usize, |sum, (index, code)| sum + index * code)
            % 103;
        if codes[codes.len() - 2] != checksum {
            return Err("checksum mismatch");
        }
        let text = codes[1..codes.len() - 2]
            .iter()
            .map(|code| {
                u8::try_from(code + 32)
                    .ok()
                    .map(char::from)
                    .ok_or("non-Code-128-B codeword")
            })
            .collect::<Result<String, _>>()?;
        Ok((codes, text))
    }

    #[test]
    fn rejects_old_format() {
        let mut d = document(vec![]);
        d.format = "piqae.business-document/v1".into();
        assert!(matches!(
            render(&d, &json!({}), RenderLimits::default()),
            Err(RenderError::UnsupportedVersion(_))
        ))
    }
    #[test]
    fn render_input_root_must_be_an_object() {
        let d = document(vec![text("bounded")]);
        for input in [json!(null), json!([{"value": 1}]), json!("value")] {
            assert_eq!(
                render(&d, &input, RenderLimits::default()),
                Err(RenderError::Invalid("render input must be a JSON object"))
            );
        }
    }
    #[test]
    fn resource_count_accepts_one_hundred_and_rejects_one_hundred_and_one() {
        let mut d = document(Vec::new());
        for index in 0..100 {
            d.resources.insert(
                format!("image_{index}"),
                Resource::Image {
                    digest: format!("sha256:{}", "a".repeat(64)),
                    media_type: "image/jpeg".into(),
                    byte_length: 1,
                },
            );
        }
        assert_eq!(validate(&d, RenderLimits::default()), Ok(()));

        d.resources.insert(
            "image_100".into(),
            Resource::Image {
                digest: format!("sha256:{}", "b".repeat(64)),
                media_type: "image/jpeg".into(),
                byte_length: 1,
            },
        );
        assert_eq!(
            validate(&d, RenderLimits::default()),
            Err(RenderError::Limit("resources"))
        );
    }
    #[test]
    fn wraps_and_paginates() {
        let d=document((0..200).map(|_|text("A long invoice line with enough words to wrap predictably across the available page width.")).collect());
        let pdf = render(&d, &json!({}), RenderLimits::default()).unwrap();
        assert!(String::from_utf8_lossy(&pdf).contains("/Count 4"));
    }
    #[test]
    fn fails_closed_when_a_column_would_paginate() {
        let child = Node::Stack {
            children: (0..100)
                .map(|_| text("A deliberately tall column row."))
                .collect(),
            gap_mm: 0.0,
        };
        let d = document(vec![Node::Row {
            children: vec![child, text("Second column")],
            gap_mm: 2.0,
        }]);
        assert_eq!(
            render(&d, &json!({}), RenderLimits::default()),
            Err(RenderError::Unsupported(
                "row and grid children cannot paginate in renderer ABI v1"
            ))
        );
    }
    #[test]
    fn table_rows_zero_one_fifty_two_hundred() {
        for count in [0, 1, 50, 200, 1_000] {
            let d = document(vec![Node::Table {
                items: Expr::Path {
                    path: vec!["items".into()],
                },
                columns: vec![TableColumn {
                    header: vec![Inline::Text {
                        value: "Item".into(),
                        style: TextStyle::default(),
                    }],
                    cell: vec![Inline::Value {
                        value: path("name"),
                        style: TextStyle::default(),
                    }],
                    width: 1.0,
                    align: TextAlign::Left,
                }],
                repeat_header: true,
                empty: vec![text("No items")],
                style: TableStyle::default(),
            }]);
            let data = json!({"items":(0..count).map(|i|json!({"name":format!("Line {i}")})).collect::<Vec<_>>()});
            assert!(
                render(&d, &data, RenderLimits::default())
                    .unwrap()
                    .starts_with(b"%PDF")
            );
        }
    }
    #[test]
    fn continuous_receipt_has_natural_height() {
        let mut d = document(vec![
            text("Receipt"),
            Node::Repeat {
                items: Expr::Path {
                    path: vec!["items".into()],
                },
                children: vec![Node::Paragraph {
                    content: vec![Inline::Value {
                        value: path("name"),
                        style: TextStyle::default(),
                    }],
                    style: TextStyle::default(),
                }],
                gap_mm: 1.0,
            },
        ]);
        d.media = Media::Continuous {
            width_mm: 80.0,
            margins: default_margins(),
        };
        let pdf = render(
            &d,
            &json!({"items":[{"name":"Coffee"},{"name":"Tea"}]}),
            RenderLimits::default(),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 226.77"));
    }

    #[test]
    fn continuous_regions_flow_without_overlapping_body_or_crop() {
        fn media_height(pdf: &[u8]) -> f32 {
            let text = String::from_utf8_lossy(pdf);
            let marker = "/MediaBox [0 0 ";
            let rest = text.split_once(marker).unwrap().1;
            rest.split_once(']')
                .unwrap()
                .0
                .split_whitespace()
                .nth(1)
                .unwrap()
                .parse()
                .unwrap()
        }

        let mut plain = document(vec![text("RECEIPT_BODY")]);
        plain.media = Media::Continuous {
            width_mm: 80.0,
            margins: default_margins(),
        };
        let plain_pdf = render(&plain, &json!({}), RenderLimits::default()).unwrap();

        let mut regions = plain;
        regions.header = Some(Region {
            first: vec![text("RECEIPT_HEADER"), Node::Spacer { height_mm: 5.0 }],
            default: vec![],
            last: vec![],
        });
        regions.footer = Some(Region {
            first: vec![],
            default: vec![],
            last: vec![Node::Spacer { height_mm: 5.0 }, text("RECEIPT_FOOTER")],
        });
        let regions_pdf = render(&regions, &json!({}), RenderLimits::default()).unwrap();
        let content = String::from_utf8_lossy(&regions_pdf);
        assert!(content.contains("(RECEIPT_HEADER)"));
        assert!(content.contains("(RECEIPT_BODY)"));
        assert!(content.contains("(RECEIPT_FOOTER)"));
        assert!(media_height(&regions_pdf) > media_height(&plain_pdf) + mm(9.0));
    }
    #[test]
    fn qr_and_code128_render() {
        let d = document(vec![
            Node::Qr {
                value: Expr::Literal {
                    value: json!("https://piqae.com"),
                },
                size_mm: 20.0,
                error_correction: QrCorrection::M,
            },
            Node::Barcode {
                value: Expr::Literal {
                    value: json!("SKU-123"),
                },
                symbology: BarcodeSymbology::Code128,
                width_mm: 50.0,
                height_mm: 15.0,
                human_readable: true,
                align: TextAlign::Center,
                padding_mm: 1.0,
                gap_mm: 1.4,
            },
        ]);
        assert!(
            render(&d, &json!({}), RenderLimits::default())
                .unwrap()
                .len()
                > 1_000
        )
    }

    #[test]
    fn barcode_alignment_padding_and_value_gap_have_exact_geometry() {
        let doc = document(vec![]);
        let root = json!({});
        let resolved = ResolvedResources::default();
        let width = mm(50.0);
        let height = mm(15.0);
        let padding = mm(2.0);
        let gap = mm(3.0);

        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            let mut state = test_state(&doc, &root, &resolved);
            let initial_y = state.y;
            let expected_footprint_x = match align {
                TextAlign::Left => state.x,
                TextAlign::Center => state.x + (state.content_width - width - padding * 2.0) / 2.0,
                TextAlign::Right => state.x + state.content_width - width - padding * 2.0,
            };
            barcode(
                &Expr::Literal {
                    value: json!("ORDER-1001"),
                },
                BarcodeLayout {
                    symbology: BarcodeSymbology::Code128,
                    width_mm: 50.0,
                    height_mm: 15.0,
                    human_readable: true,
                    align,
                    padding_mm: 2.0,
                    gap_mm: 3.0,
                },
                &mut state,
            )
            .unwrap();

            let bars = state.pages[0]
                .draws
                .iter()
                .find_map(|draw| match draw {
                    Draw::Bars {
                        x,
                        y,
                        width,
                        height,
                        ..
                    } => Some((*x, *y, *width, *height)),
                    _ => None,
                })
                .unwrap();
            assert!((bars.0 - (expected_footprint_x + padding)).abs() < 0.001);
            assert!((bars.1 - (initial_y - padding - height)).abs() < 0.001);
            assert!((bars.2 - width).abs() < 0.001);
            assert!((bars.3 - height).abs() < 0.001);

            let label = state.pages[0]
                .draws
                .iter()
                .find_map(|draw| match draw {
                    Draw::Text {
                        x, y, text, size, ..
                    } if text == "ORDER-1001" => Some((*x, *y, *size)),
                    _ => None,
                })
                .unwrap();
            let label_width = text_width("ORDER-1001", doc.theme.font_size_pt, FontFace::Regular);
            assert!(
                (label.0 - (expected_footprint_x + (width + padding * 2.0 - label_width) / 2.0))
                    .abs()
                    < 0.001
            );
            assert!(
                (label.1 - (initial_y - padding - height - gap - doc.theme.font_size_pt)).abs()
                    < 0.001
            );
            assert!((label.2 - doc.theme.font_size_pt).abs() < 0.001);
            let expected_consumed =
                padding * 2.0 + height + gap + doc.theme.font_size_pt * doc.theme.line_height;
            assert!((state.y - (initial_y - expected_consumed)).abs() < 0.001);
        }
    }

    #[test]
    fn qr_geometry_and_inter_element_gaps_do_not_gain_editor_only_space() {
        let doc = document(vec![]);
        let root = json!({});
        let resolved = ResolvedResources::default();
        let mut qr_state = test_state(&doc, &root, &resolved);
        let initial_qr_y = qr_state.y;
        qr(
            &Expr::Literal {
                value: json!("https://piqae.com/orders/1001"),
            },
            24.0,
            QrCorrection::Q,
            &mut qr_state,
        )
        .unwrap();
        let qr_draw = qr_state.pages[0]
            .draws
            .iter()
            .find_map(|draw| match draw {
                Draw::Qr { x, y, size, .. } => Some((*x, *y, *size)),
                _ => None,
            })
            .unwrap();
        assert!((qr_draw.0 - qr_state.x).abs() < 0.001);
        assert!((qr_draw.1 - (initial_qr_y - mm(24.0))).abs() < 0.001);
        assert!((qr_draw.2 - mm(24.0)).abs() < 0.001);

        let mut stack_state = test_state(&doc, &root, &resolved);
        let initial_stack_y = stack_state.y;
        layout_nodes(
            &[Node::Stack {
                children: vec![
                    Node::Spacer { height_mm: 2.0 },
                    Node::Spacer { height_mm: 3.0 },
                ],
                gap_mm: 4.0,
            }],
            &mut stack_state,
            0,
        )
        .unwrap();
        assert!((stack_state.y - (initial_stack_y - mm(9.0))).abs() < 0.001);
    }

    #[test]
    fn code128_has_mandatory_quiet_zones_and_decodes_with_checksum() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../standards/printpacket/conformance/code128-decode.json"
        ))
        .unwrap();
        let value = fixture["value"].as_str().unwrap();
        let quiet = fixture["quiet_zone_modules"].as_u64().unwrap() as usize;
        let expected_codes = fixture["expected_codewords"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_u64().unwrap() as usize)
            .collect::<Vec<_>>();
        let bits = code128_bits(value);
        let (codes, decoded) = decode_code128_b(&bits).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(codes, expected_codes);
        assert_eq!(&bits[..quiet], vec![false; quiet]);
        assert_eq!(&bits[bits.len() - quiet..], vec![false; quiet]);
    }
    #[test]
    fn bounded_and_explicit_unsupported() {
        let d = document(vec![text("\u{65e5}\u{672c}")]);
        assert!(matches!(
            render(&d, &json!({}), RenderLimits::default()),
            Err(RenderError::UnsupportedCharacter { .. })
        ));
        let mut limits = RenderLimits::default();
        limits.max_repeat_items = 1;
        let d = document(vec![Node::Repeat {
            items: Expr::Path {
                path: vec!["x".into()],
            },
            children: vec![text("x")],
            gap_mm: 0.0,
        }]);
        assert_eq!(
            render(&d, &json!({"x":[1,2]}), limits),
            Err(RenderError::Limit("repeat items"))
        );
    }

    #[test]
    fn repeated_headers_and_footers_are_emitted_per_page() {
        let mut d = document((0..180).map(|_| text("flowing body line")).collect());
        d.header = Some(Region {
            first: vec![text("FIRST HEADER")],
            default: vec![text("PAGE HEADER")],
            last: vec![],
        });
        d.footer = Some(Region {
            first: vec![],
            default: vec![text("PAGE FOOTER")],
            last: vec![text("LAST FOOTER")],
        });
        let pdf =
            String::from_utf8(render(&d, &json!({}), RenderLimits::default()).unwrap()).unwrap();
        assert!(pdf.matches("(PAGE FOOTER)").count() >= 3);
        assert_eq!(pdf.matches("(LAST FOOTER)").count(), 1);
        assert_eq!(pdf.matches("(FIRST HEADER)").count(), 1);
        assert!(pdf.contains("(PAGE HEADER)"));
    }

    #[test]
    fn label_and_expression_profile() {
        let mut d = document(vec![Node::Conditional {
            condition: Expr::Compare {
                operator: CompareOperator::Greater,
                left: Box::new(Expr::Path {
                    path: vec!["quantity".into()],
                }),
                right: Box::new(Expr::Literal { value: json!(1) }),
            },
            then: vec![Node::Paragraph {
                content: vec![Inline::Value {
                    value: Expr::Concat {
                        values: vec![
                            Expr::Literal {
                                value: json!("Batch "),
                            },
                            Expr::Path {
                                path: vec!["batch".into()],
                            },
                        ],
                    },
                    style: TextStyle::default(),
                }],
                style: TextStyle::default(),
            }],
            otherwise: vec![],
        }]);
        d.media = Media::Label {
            width_mm: 50.0,
            height_mm: 30.0,
            margins: Edges {
                top_mm: 2.0,
                right_mm: 2.0,
                bottom_mm: 2.0,
                left_mm: 2.0,
            },
        };
        let pdf = render(
            &d,
            &json!({"quantity": 2, "batch": "LOT-42"}),
            RenderLimits::default(),
        )
        .unwrap();
        let pdf = String::from_utf8_lossy(&pdf);
        assert!(pdf.contains("(Batch LOT-42)"));
    }

    #[test]
    fn continuous_media_rejects_explicit_page_break() {
        let mut d = document(vec![Node::PageBreak]);
        d.media = Media::Continuous {
            width_mm: 58.0,
            margins: default_margins(),
        };
        assert_eq!(
            render(&d, &json!({}), RenderLimits::default()),
            Err(RenderError::Unsupported("page breaks on continuous media"))
        );
    }

    #[test]
    fn repeated_orders_do_not_emit_a_trailing_blank_page() {
        let d = document(vec![Node::Repeat {
            items: Expr::Path {
                path: vec!["orders".into()],
            },
            children: vec![text("packing slip"), Node::PageBreak],
            gap_mm: 0.0,
        }]);
        let orders = (0..250).map(|_| json!({})).collect::<Vec<_>>();
        let output = render_with_metrics(
            &d,
            &json!({"orders": orders}),
            &ResolvedResources::default(),
            RenderLimits::default(),
        )
        .unwrap();
        assert_eq!(output.page_count, 250);
    }

    #[test]
    fn keep_together_rejects_dynamic_content_that_would_split_across_labels() {
        let mut d = document(vec![Node::KeepTogether {
            children: vec![Node::Paragraph {
                content: vec![Inline::Value {
                    value: Expr::Path {
                        path: vec!["description".into()],
                    },
                    style: TextStyle::default(),
                }],
                style: TextStyle::default(),
            }],
        }]);
        d.media = Media::Label {
            width_mm: 50.0,
            height_mm: 30.0,
            margins: Edges {
                top_mm: 2.0,
                right_mm: 2.0,
                bottom_mm: 2.0,
                left_mm: 2.0,
            },
        };
        let description = std::iter::repeat_n("word", 30)
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            render(
                &d,
                &json!({"description": description}),
                RenderLimits::default()
            ),
            Err(RenderError::Limit("keep-together block height"))
        );
    }

    #[test]
    fn deferred_page_break_does_not_reenter_footer_layout() {
        let mut d = document(vec![text("first"), Node::PageBreak, text("second")]);
        d.footer = Some(Region {
            first: vec![],
            default: vec![text("footer")],
            last: vec![],
        });
        let output = render_with_metrics(
            &d,
            &json!({}),
            &ResolvedResources::default(),
            RenderLimits::default(),
        )
        .unwrap();
        assert_eq!(output.page_count, 2);
    }

    #[test]
    fn mixed_inline_styles_and_date_format_are_preserved() {
        let d = document(vec![Node::Paragraph {
            content: vec![
                Inline::Text {
                    value: "Issued ".into(),
                    style: TextStyle::default(),
                },
                Inline::Value {
                    value: Expr::FormatDate {
                        value: Box::new(Expr::Path {
                            path: vec!["issued_at".into()],
                        }),
                        format: DateFormat::DayMonthYear,
                    },
                    style: TextStyle {
                        bold: true,
                        underline: true,
                        font_size_pt: Some(12.0),
                        ..Default::default()
                    },
                },
            ],
            style: TextStyle::default(),
        }]);
        let pdf = String::from_utf8(
            render(
                &d,
                &json!({"issued_at":"2026-08-19T10:00:00Z"}),
                RenderLimits::default(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(pdf.contains("Issued"));
        assert!(pdf.contains("19/08/2026"));
        assert!(pdf.contains("/F2 12.00 Tf"));
    }

    #[test]
    fn inline_whitespace_is_preserved_without_fragmenting_same_style_text() {
        let style = StyledRun {
            text: String::new(),
            size: 10.0,
            face: FontFace::Regular,
            underline: false,
            line_break: false,
            color: Color::default(),
        };
        let lines = wrap_runs(
            vec![
                StyledRun {
                    text: "PACKING   ".into(),
                    ..style.clone()
                },
                StyledRun {
                    text: "SLIP".into(),
                    ..style.clone()
                },
                StyledRun {
                    text: "\nBen Smith\r\n8 Iris Taylor Avenue".into(),
                    ..style
                },
            ],
            500.0,
        )
        .unwrap();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].text, "PACKING SLIP");
        assert_eq!(lines[1].len(), 1);
        assert_eq!(lines[1][0].text, "Ben Smith");
        assert_eq!(lines[2].len(), 1);
        assert_eq!(lines[2][0].text, "8 Iris Taylor Avenue");
    }

    #[test]
    fn adjacent_inline_runs_do_not_gain_an_implicit_space() {
        let style = StyledRun {
            text: String::new(),
            size: 10.0,
            face: FontFace::Bold,
            underline: false,
            line_break: false,
            color: Color::default(),
        };
        let lines = wrap_runs(
            vec![
                StyledRun {
                    text: "SHIP".into(),
                    ..style.clone()
                },
                StyledRun {
                    text: "TO".into(),
                    ..style
                },
            ],
            500.0,
        )
        .unwrap();

        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].text, "SHIPTO");
    }

    #[test]
    fn win_ansi_text_uses_base14_helvetica_glyph_widths() {
        assert_eq!(helvetica_width('\u{00c6}', false), 1_000);
        assert_eq!(helvetica_width('\u{0153}', true), 944);
        assert_eq!(helvetica_width('\u{00df}', false), 611);
        assert_eq!(helvetica_width('\u{00f8}', false), 611);
        assert_eq!(helvetica_width('\u{017d}', true), 611);
        assert_eq!(helvetica_width('\u{017e}', true), 500);
        assert_eq!(helvetica_width('\u{2014}', false), 1_000);
        assert_eq!(helvetica_width('\u{201c}', false), 333);
        assert_eq!(helvetica_width('\u{201c}', true), 500);
    }

    #[test]
    fn resolved_jpeg_is_digest_verified_and_embedded() {
        // Minimal bounded SOF fixture. The renderer treats JPEG entropy as an
        // opaque DCT stream; production hosts additionally decode at ingestion.
        let jpeg = vec![
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11,
            0x00, 0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xd9,
        ];
        let digest = format!("sha256:{:x}", Sha256::digest(&jpeg));
        let mut d = document(vec![Node::Image {
            resource: "logo".into(),
            width_mm: 20.0,
            height_mm: 10.0,
            fit: ImageFit::Contain,
        }]);
        d.resources.insert(
            "logo".into(),
            Resource::Image {
                digest,
                media_type: "image/jpeg".into(),
                byte_length: jpeg.len() as u64,
            },
        );
        let resolved = ResolvedResources {
            images: BTreeMap::from([("logo".into(), jpeg)]),
        };
        let pdf =
            render_with_resources(&d, &json!({}), &resolved, RenderLimits::default()).unwrap();
        assert!(pdf.windows(10).any(|window| window == b"/DCTDecode"));
        d.body = vec![Node::ImageValue {
            resource: Expr::Path {
                path: vec!["product_image".into()],
            },
            width_mm: 20.0,
            height_mm: 10.0,
            fit: ImageFit::Contain,
        }];
        let selected = render_with_resources(
            &d,
            &json!({"product_image": "logo"}),
            &resolved,
            RenderLimits::default(),
        )
        .unwrap();
        assert!(selected.windows(10).any(|window| window == b"/DCTDecode"));
        let mut wrong = resolved;
        wrong.images.get_mut("logo").unwrap()[2] = 0;
        assert_eq!(
            render_with_resources(
                &d,
                &json!({"product_image": "logo"}),
                &wrong,
                RenderLimits::default()
            ),
            Err(RenderError::Invalid("image digest mismatch"))
        );
    }

    #[test]
    fn repeated_jpeg_occurrences_share_one_bounded_pdf_object() {
        let jpeg = vec![
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11,
            0x00, 0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xd9,
        ];
        let mut document = document(
            (0..100)
                .map(|_| Node::Image {
                    resource: "logo".into(),
                    width_mm: 20.0,
                    height_mm: 2.0,
                    fit: ImageFit::Contain,
                })
                .collect(),
        );
        document.resources.insert(
            "logo".into(),
            Resource::Image {
                digest: format!("sha256:{:x}", Sha256::digest(&jpeg)),
                media_type: "image/jpeg".into(),
                byte_length: jpeg.len() as u64,
            },
        );
        let pdf = render_with_resources(
            &document,
            &json!({}),
            &ResolvedResources {
                images: BTreeMap::from([("logo".into(), jpeg)]),
            },
            RenderLimits::default(),
        )
        .unwrap();
        assert_eq!(
            pdf.windows(10)
                .filter(|window| *window == b"/DCTDecode")
                .count(),
            1
        );
        assert_eq!(
            pdf.windows(3).filter(|window| *window == b" Do").count(),
            100
        );
    }

    #[test]
    fn aggregate_resources_and_incremental_output_are_bounded_before_pdf_growth() {
        let mut resource_document = document(vec![]);
        for index in 0..4 {
            resource_document.resources.insert(
                format!("image-{index}"),
                Resource::Image {
                    digest:
                        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                            .into(),
                    media_type: "image/jpeg".into(),
                    byte_length: 4 * 1024 * 1024,
                },
            );
        }
        assert_eq!(
            validate(&resource_document, RenderLimits::default()),
            Err(RenderError::Limit("resource bytes"))
        );

        let tiny = RenderLimits {
            max_output_bytes: 512,
            ..RenderLimits::default()
        };
        assert_eq!(
            render(&document(vec![text("bounded")]), &json!({}), tiny),
            Err(RenderError::Limit("output bytes"))
        );
    }

    #[test]
    fn every_declared_resource_is_verified_even_when_not_laid_out() {
        let jpeg = vec![
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11,
            0x00, 0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xd9,
        ];
        let mut document = document(vec![Node::Paragraph {
            content: vec![Inline::Text {
                value: "No image node".into(),
                style: TextStyle::default(),
            }],
            style: TextStyle::default(),
        }]);
        document.resources.insert(
            "unused".into(),
            Resource::Image {
                digest: format!("sha256:{:x}", Sha256::digest(&jpeg)),
                media_type: "image/jpeg".into(),
                byte_length: jpeg.len() as u64,
            },
        );
        assert_eq!(
            render_with_resources(
                &document,
                &json!({}),
                &ResolvedResources::default(),
                RenderLimits::default()
            ),
            Err(RenderError::Invalid("resolved resource set mismatch"))
        );
        let mut corrupt = jpeg;
        corrupt[3] ^= 1;
        assert_eq!(
            render_with_resources(
                &document,
                &json!({}),
                &ResolvedResources {
                    images: BTreeMap::from([("unused".into(), corrupt)])
                },
                RenderLimits::default()
            ),
            Err(RenderError::Invalid("image digest mismatch"))
        );
    }

    #[test]
    fn branded_box_and_table_styles_render_deterministically() {
        let navy = Color {
            red: 0,
            green: 54,
            blue: 110,
        };
        let white = Color {
            red: 255,
            green: 255,
            blue: 255,
        };
        let d = document(vec![
            Node::Box {
                children: vec![Node::Heading {
                    content: vec![Inline::Text {
                        value: "PACKING SLIP".into(),
                        style: TextStyle::default(),
                    }],
                    level: 2,
                    style: TextStyle {
                        color: Some(white),
                        ..Default::default()
                    },
                }],
                style: BoxStyle {
                    padding_mm: 4.0,
                    background: Some(navy),
                    border_color: Some(navy),
                    border_width_pt: 1.0,
                },
            },
            Node::Table {
                items: Expr::Path {
                    path: vec!["items".into()],
                },
                columns: vec![TableColumn {
                    header: vec![Inline::Text {
                        value: "ITEM".into(),
                        style: TextStyle::default(),
                    }],
                    cell: vec![Inline::Value {
                        value: Expr::CurrentPath {
                            path: vec!["name".into()],
                        },
                        style: TextStyle::default(),
                    }],
                    width: 1.0,
                    align: TextAlign::Left,
                }],
                repeat_header: true,
                empty: vec![],
                style: TableStyle {
                    header_background: Some(navy),
                    header_text_color: Some(white),
                    ..TableStyle::default()
                },
            },
        ]);
        let input = json!({"items": [{"name": "Coffee"}]});
        let first = render(&d, &input, RenderLimits::default()).unwrap();
        let second = render(&d, &input, RenderLimits::default()).unwrap();
        assert_eq!(first, second);
        let pdf = String::from_utf8(first).unwrap();
        assert!(pdf.contains("0.0000 0.2118 0.4314 rg"));
        assert!(pdf.contains("1.0000 1.0000 1.0000 rg"));
        assert!(pdf.contains(" re f"));
    }

    #[test]
    fn decoration_dimensions_are_bounded() {
        let d = document(vec![Node::Box {
            children: vec![text("x")],
            style: BoxStyle {
                padding_mm: 51.0,
                ..BoxStyle::default()
            },
        }]);
        assert_eq!(
            validate(&d, RenderLimits::default()),
            Err(RenderError::Invalid("box style"))
        );
    }

    #[test]
    fn validation_rejects_render_time_only_layout_failures() {
        let invalid_grid = document(vec![Node::Grid {
            columns: vec![1.0],
            children: vec![text("one"), text("two")],
            gap_mm: 1.0,
        }]);
        assert_eq!(
            validate(&invalid_grid, RenderLimits::default()),
            Err(RenderError::Invalid("grid columns"))
        );

        let unsupported_fit = document(vec![Node::Image {
            resource: "photo".into(),
            width_mm: 20.0,
            height_mm: 20.0,
            fit: ImageFit::Cover,
        }]);
        assert_eq!(
            validate(&unsupported_fit, RenderLimits::default()),
            Err(RenderError::Unsupported(
                "image cover cropping is not available in renderer ABI v1"
            ))
        );

        let invalid_operands = document(vec![Node::Conditional {
            condition: Expr::Boolean {
                operator: BooleanOperator::And,
                values: (0..101)
                    .map(|_| Expr::Literal { value: json!(true) })
                    .collect(),
            },
            then: vec![],
            otherwise: vec![],
        }]);
        assert_eq!(
            validate(&invalid_operands, RenderLimits::default()),
            Err(RenderError::Limit("expression operands"))
        );

        let mut nested_break = document(vec![Node::KeepTogether {
            children: vec![Node::Conditional {
                condition: Expr::Literal { value: json!(true) },
                then: vec![Node::PageBreak],
                otherwise: vec![],
            }],
        }]);
        nested_break.media = Media::Continuous {
            width_mm: 80.0,
            margins: default_margins(),
        };
        assert_eq!(
            validate(&nested_break, RenderLimits::default()),
            Err(RenderError::Unsupported("page breaks on continuous media"))
        );
    }

    #[test]
    fn tallest_region_variant_reserves_body_space() {
        let mut packet = document(vec![Node::Spacer { height_mm: 15.0 }]);
        packet.media = Media::Label {
            width_mm: 50.0,
            height_mm: 30.0,
            margins: Edges {
                top_mm: 2.0,
                right_mm: 2.0,
                bottom_mm: 2.0,
                left_mm: 2.0,
            },
        };
        packet.header = Some(Region {
            first: vec![Node::Spacer { height_mm: 15.0 }],
            default: vec![Node::Spacer { height_mm: 1.0 }],
            last: vec![],
        });
        assert_eq!(
            render(
                &packet,
                &json!({}),
                RenderLimits {
                    max_pages: 1,
                    ..RenderLimits::default()
                }
            ),
            Err(RenderError::Limit("pages"))
        );
    }

    #[test]
    fn optional_paths_and_membership_rules_are_safe() {
        let missing = Expr::Path {
            path: vec!["product".into(), "metafields".into(), "optional".into()],
        };
        assert_eq!(
            eval(
                &Expr::Exists {
                    value: Box::new(missing.clone()),
                },
                &json!({"product": {}}),
                &json!({}),
            )
            .unwrap(),
            json!(false)
        );
        assert_eq!(
            eval(
                &Expr::Coalesce {
                    values: vec![
                        missing,
                        Expr::Literal {
                            value: json!("fallback")
                        }
                    ],
                },
                &json!({"product": {}}),
                &json!({}),
            )
            .unwrap(),
            json!("fallback")
        );
        assert_eq!(
            eval(
                &Expr::Contains {
                    collection: Box::new(Expr::Path {
                        path: vec!["category".into(), "ancestorIds".into()],
                    }),
                    value: Box::new(Expr::Literal {
                        value: json!("gid://shopify/TaxonomyCategory/aa")
                    }),
                },
                &json!({"category": {"ancestorIds": ["gid://shopify/TaxonomyCategory/aa"]}}),
                &json!({}),
            )
            .unwrap(),
            json!(true)
        );
    }

    #[test]
    fn rich_data_list_repeats_designed_header_and_keeps_items_atomic() {
        let header = Node::Box {
            children: vec![text("ITEM DESCRIPTION")],
            style: BoxStyle {
                padding_mm: 2.0,
                background: Some(Color {
                    red: 8,
                    green: 50,
                    blue: 96,
                }),
                ..BoxStyle::default()
            },
        };
        let item = Node::Grid {
            columns: vec![3.0, 1.0],
            gap_mm: 3.0,
            children: vec![
                Node::Stack {
                    children: vec![Node::Paragraph {
                        content: vec![Inline::Value {
                            value: Expr::CurrentPath {
                                path: vec!["title".into()],
                            },
                            style: TextStyle {
                                bold: true,
                                ..Default::default()
                            },
                        }],
                        style: TextStyle::default(),
                    }],
                    gap_mm: 1.0,
                },
                Node::Barcode {
                    value: Expr::CurrentPath {
                        path: vec!["barcode".into()],
                    },
                    symbology: BarcodeSymbology::Code128,
                    width_mm: 35.0,
                    height_mm: 10.0,
                    human_readable: false,
                    align: TextAlign::Left,
                    padding_mm: 0.0,
                    gap_mm: 1.4,
                },
            ],
        };
        let d = document(vec![Node::DataList {
            items: Expr::Path {
                path: vec!["items".into()],
            },
            header: vec![header],
            item: vec![item],
            empty: vec![text("No items")],
            repeat_header: true,
            gap_mm: 2.0,
        }]);
        let items = (0..120)
            .map(|index| json!({"title": format!("Coffee {index}"), "barcode": format!("SKU-{index:04}")}))
            .collect::<Vec<_>>();
        let output = render_with_metrics(
            &d,
            &json!({"items": items}),
            &ResolvedResources::default(),
            RenderLimits::default(),
        )
        .unwrap();
        assert!(output.page_count > 1);
        let pdf = String::from_utf8(output.pdf).unwrap();
        assert_eq!(
            pdf.matches("(ITEM DESCRIPTION) Tj").count(),
            output.page_count as usize
        );
        assert_eq!(pdf.matches("(Coffee ").count(), 120);
    }

    #[test]
    fn renderer_owned_page_numbers_and_count_are_resolved_per_page() {
        let mut d = document((0..180).map(|_| text("flowing body line")).collect());
        d.footer = Some(Region {
            first: vec![],
            default: vec![Node::Paragraph {
                content: vec![
                    Inline::Value {
                        value: Expr::PageNumber,
                        style: TextStyle::default(),
                    },
                    Inline::Text {
                        value: " of ".into(),
                        style: TextStyle::default(),
                    },
                    Inline::Value {
                        value: Expr::PageCount,
                        style: TextStyle::default(),
                    },
                ],
                style: TextStyle::default(),
            }],
            last: vec![],
        });
        let output = render_with_metrics(
            &d,
            &json!({}),
            &ResolvedResources::default(),
            RenderLimits::default(),
        )
        .unwrap();
        let count = output.page_count;
        let pdf = String::from_utf8(output.pdf).unwrap();
        assert!(pdf.contains(&format!("(1 of {count}) Tj")));
        assert!(pdf.contains(&format!("({count} of {count}) Tj")));
        assert!(!pdf.contains(PAGE_NUMBER_MARKER));
        assert!(!pdf.contains(PAGE_COUNT_MARKER));
        assert_eq!(
            eval(&Expr::PageNumber, &json!({}), &json!({})),
            Err(RenderError::Unsupported(
                "page context expressions are supported only as direct inline values"
            ))
        );
    }

    #[test]
    fn expression_depth_operands_paths_and_literals_are_bounded_before_render() {
        let mut expression = Expr::Literal { value: json!(true) };
        for _ in 0..66 {
            expression = Expr::Not {
                value: Box::new(expression),
            };
        }
        let deep = document(vec![Node::Conditional {
            condition: expression,
            then: vec![text("unsafe")],
            otherwise: vec![],
        }]);
        assert_eq!(
            validate(&deep, RenderLimits::default()),
            Err(RenderError::Limit("expression depth"))
        );

        let wide = document(vec![Node::Paragraph {
            content: vec![Inline::Value {
                value: Expr::Concat {
                    values: (0..1_025)
                        .map(|_| Expr::Literal { value: json!("x") })
                        .collect(),
                },
                style: TextStyle::default(),
            }],
            style: TextStyle::default(),
        }]);
        assert_eq!(
            validate(&wide, RenderLimits::default()),
            Err(RenderError::Limit("expression operands"))
        );

        let path = document(vec![Node::Paragraph {
            content: vec![Inline::Value {
                value: Expr::Path {
                    path: vec!["x".repeat(121)],
                },
                style: TextStyle::default(),
            }],
            style: TextStyle::default(),
        }]);
        assert_eq!(
            validate(&path, RenderLimits::default()),
            Err(RenderError::Limit("expression path"))
        );

        let literal = document(vec![Node::Conditional {
            condition: Expr::Literal {
                value: json!("x".repeat(1024 * 1024 + 1)),
            },
            then: vec![],
            otherwise: vec![],
        }]);
        assert_eq!(
            validate(&literal, RenderLimits::default()),
            Err(RenderError::Limit("expression literal bytes"))
        );
    }

    #[test]
    fn every_region_variant_is_reserved_and_rendered_exactly_once_when_selected() {
        let mut packet = document((0..180).map(|_| text("flowing body line")).collect());
        packet.header = Some(Region {
            first: vec![text("HEADER_FIRST")],
            default: vec![text("HEADER_DEFAULT")],
            last: vec![
                text("HEADER_LAST"),
                text("last header is deliberately taller"),
                text("and body space reserves its maximum height"),
            ],
        });
        packet.footer = Some(Region {
            first: vec![text("FOOTER_FIRST"), text("first footer is taller")],
            default: vec![text("FOOTER_DEFAULT")],
            last: vec![
                text("FOOTER_LAST"),
                text("last footer is deliberately taller"),
                text("and never overlaps the final body"),
            ],
        });
        let output = render_with_metrics(
            &packet,
            &json!({}),
            &ResolvedResources::default(),
            RenderLimits::default(),
        )
        .unwrap();
        assert!(output.page_count > 2);
        let pdf = String::from_utf8(output.pdf).unwrap();
        assert_eq!(pdf.matches("(HEADER_FIRST)").count(), 1);
        assert_eq!(pdf.matches("(HEADER_LAST)").count(), 1);
        assert_eq!(
            pdf.matches("(HEADER_DEFAULT)").count(),
            output.page_count as usize - 2
        );
        assert_eq!(pdf.matches("(FOOTER_FIRST)").count(), 1);
        assert_eq!(pdf.matches("(FOOTER_LAST)").count(), 1);
        assert_eq!(
            pdf.matches("(FOOTER_DEFAULT)").count(),
            output.page_count as usize - 2
        );
    }
}
