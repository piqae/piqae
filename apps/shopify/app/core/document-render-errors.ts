/**
 * A renderer-declared terminal failure whose code is safe operational
 * metadata. The document input and upstream response body are deliberately
 * excluded so production diagnostics cannot copy merchant data into logs.
 */
export class DocumentRenderFailedError extends Error {
  override readonly name = "DocumentRenderFailedError";
  readonly failureCode: string;

  constructor(
    failureCode: string,
    resource: "document" | "product label" | "PDF preview" = "document",
  ) {
    super(`${resource} render failed`);
    this.failureCode = SAFE_DOCUMENT_RENDER_FAILURE_CODES.has(failureCode)
      ? failureCode
      : "unknown_render_failure";
  }
}

const SAFE_DOCUMENT_RENDER_FAILURE_CODES = new Set([
  "document_artifact_store_failed",
  "document_artifact_too_large",
  "document_barcode_invalid",
  "document_data_invalid",
  "document_data_missing",
  "document_decryption_failed",
  "document_encryption_failed",
  "document_page_count_invalid",
  "document_render_failed",
  "document_render_limit_exceeded",
  "document_resource_aggregate_too_large",
  "document_resource_integrity_failed",
  "document_resource_timeout",
  "document_resource_too_large",
  "document_resource_unavailable",
  "document_typography_unsupported",
  "failed_retryable",
  "failed_terminal",
  "invalid_document_input",
  "invalid_document_spec",
  "preview_expired",
  "render_lease_lost",
  "render_timeout",
  "render_worker_panic",
  "renderer_capacity_timeout",
  "renderer_feature_unsupported",
  "renderer_unavailable",
  "renderer_version_unsupported",
]);
