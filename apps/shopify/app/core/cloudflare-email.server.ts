const MAX_PDF_BYTES = 3_500_000;
const MAX_MESSAGE_BYTES = 5 * 1024 * 1024;
export class EmailDeliveryError extends Error {
  constructor(
    message: string,
    readonly retryable: boolean,
  ) {
    super(message);
    this.name = "EmailDeliveryError";
  }
}
export interface TransactionalEmail {
  to: string;
  subject: string;
  html: string;
  text: string;
  pdf: Uint8Array;
  filename: string;
}
export interface CloudflareEmailOptions {
  accountId: string;
  token: string;
  fromAddress: string;
  fromName: string;
  replyTo?: string;
  timeoutMs?: number;
  fetch?: typeof fetch;
}

export class CloudflareEmailClient {
  private readonly fetcher: typeof fetch;
  constructor(private readonly options: CloudflareEmailOptions) {
    this.fetcher = options.fetch ?? fetch;
    validateAddress(options.fromAddress);
    if (options.replyTo) validateAddress(options.replyTo);
  }
  async send(message: TransactionalEmail): Promise<"delivered" | "queued"> {
    validateAddress(message.to);
    if (message.pdf.byteLength > MAX_PDF_BYTES)
      throw new EmailDeliveryError(
        "email attachment exceeds size limit",
        false,
      );
    const controller = new AbortController();
    const timeout = setTimeout(
      () => controller.abort(),
      this.options.timeoutMs ?? 10_000,
    );
    let response: Response;
    try {
      const requestBody = JSON.stringify({
        to: message.to,
        from: {
          address: this.options.fromAddress,
          name: this.options.fromName,
        },
        reply_to: this.options.replyTo,
        subject: message.subject.slice(0, 200),
        html: message.html,
        text: message.text,
        attachments: [
          {
            content: Buffer.from(message.pdf).toString("base64"),
            filename: safeFilename(message.filename),
            type: "application/pdf",
            disposition: "attachment",
          },
        ],
      });
      if (Buffer.byteLength(requestBody) > MAX_MESSAGE_BYTES)
        throw new EmailDeliveryError("email message exceeds size limit", false);
      response = await this.fetcher(
        `https://api.cloudflare.com/client/v4/accounts/${encodeURIComponent(this.options.accountId)}/email/sending/send`,
        {
          method: "POST",
          signal: controller.signal,
          headers: {
            authorization: `Bearer ${this.options.token}`,
            "content-type": "application/json",
          },
          body: requestBody,
        },
      );
    } catch (error) {
      if (error instanceof EmailDeliveryError) throw error;
      throw new EmailDeliveryError(
        error instanceof DOMException && error.name === "AbortError"
          ? "email request timed out"
          : "email transport failed",
        true,
      );
    } finally {
      clearTimeout(timeout);
    }
    const body = (await response.json().catch(() => null)) as any;
    if (!response.ok || body?.success === false) {
      const retryable = response.status === 429 || response.status >= 500;
      throw new EmailDeliveryError(
        `email service rejected request (status ${response.status})`,
        retryable,
      );
    }
    if (
      Array.isArray(body?.result?.delivered) &&
      body.result.delivered.length > 0
    )
      return "delivered";
    if (Array.isArray(body?.result?.queued) && body.result.queued.length > 0)
      return "queued";
    throw new EmailDeliveryError(
      "email service returned no accepted recipients",
      false,
    );
  }
}
function validateAddress(value: string) {
  if (value.length > 320 || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value))
    throw new EmailDeliveryError("invalid email address", false);
}
function safeFilename(value: string) {
  const cleaned = value.replace(/[^a-zA-Z0-9._-]/g, "-").slice(0, 100);
  return cleaned.toLowerCase().endsWith(".pdf")
    ? cleaned
    : `${cleaned || "document"}.pdf`;
}
