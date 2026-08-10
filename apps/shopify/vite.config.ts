import { reactRouter } from "@react-router/dev/vite";
import { defineConfig } from "vite";

const assignedPort = Number.parseInt(process.env.PORT ?? "5173", 10);
const appUrl = process.env.APP_URL ?? process.env.HOST;

if (
  !Number.isInteger(assignedPort) ||
  assignedPort < 1 ||
  assignedPort > 65_535
) {
  throw new Error("PORT must be an integer between 1 and 65535");
}

const allowedHosts = appUrl ? [new URL(appUrl).hostname] : [];

export default defineConfig({
  plugins: [reactRouter()],
  resolve: {
    dedupe: ["react", "react-dom", "react-router"],
  },
  server: {
    host: "127.0.0.1",
    port: assignedPort,
    strictPort: true,
    allowedHosts,
  },
});
