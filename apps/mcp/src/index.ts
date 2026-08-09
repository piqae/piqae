#!/usr/bin/env node
import { realpathSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { loadConfig } from "./config.js";
import { startHttpServer } from "./http.js";
import { createPiqaeMcpServer } from "./server.js";

export { createPiqaeMcpServer } from "./server.js";
export { loadConfig } from "./config.js";

async function main(): Promise<void> {
  const arguments_ = new Set(process.argv.slice(2));
  if (arguments_.has("--help")) {
    process.stdout.write(
      "piqae-mcp [--stdio | --http]\n\n--stdio  Local MCP over standard input/output (default)\n--http   Stateless Streamable HTTP with bearer verification and OAuth discovery\n",
    );
    return;
  }
  if (arguments_.has("--stdio") && arguments_.has("--http")) {
    throw new Error("Choose either --stdio or --http.");
  }
  const config = loadConfig();
  if (arguments_.has("--http")) {
    const close = await startHttpServer(config);
    const shutdown = () => {
      void close().finally(() => process.exit(0));
    };
    process.once("SIGINT", shutdown);
    process.once("SIGTERM", shutdown);
    return;
  }
  const server = createPiqaeMcpServer(config);
  await server.connect(new StdioServerTransport());
  console.error("Piqae MCP stdio server ready");
}

if (
  process.argv[1] &&
  realpathSync(process.argv[1]) === realpathSync(new URL(import.meta.url))
) {
  main().catch((error: unknown) => {
    console.error(
      `Piqae MCP failed: ${error instanceof Error ? error.message : "unknown error"}`,
    );
    process.exitCode = 1;
  });
}
