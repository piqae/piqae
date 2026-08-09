import { type RouteConfig, index, route } from "@react-router/dev/routes";

export default [
  route("auth/*", "routes/auth.$.tsx"),
  route("webhooks", "routes/webhooks.tsx"),
  route("api/print/admin", "routes/api.print.admin.tsx"),
  route("api/print/admin/options", "routes/api.print.admin-options.tsx"),
  route("api/print/admin/previews", "routes/api.print.admin-previews.tsx"),
  route(
    "api/print/previews/:previewId/approve",
    "routes/api.print.preview-approve.tsx",
  ),
  route(
    "api/print/previews/:previewId/cancel",
    "routes/api.print.preview-cancel.tsx",
  ),
  route(
    "api/print/previews/:previewId/artifact",
    "routes/api.print.preview-artifact.tsx",
  ),
  route("api/print/admin-drafts", "routes/api.print.admin-drafts.tsx"),
  route("api/print/pos", "routes/api.print.pos.tsx"),
  route(
    "api/customer/renders/:renderId",
    "routes/api.customer.renders.$renderId.tsx",
  ),
  route("api/customer/documents", "routes/api.customer.documents.tsx"),
  route(
    "api/public/documents/download",
    "routes/api.public.documents.download.tsx",
  ),
  route(
    "api/public/previews/artifact",
    "routes/api.public.preview-artifact.tsx",
  ),
  route(
    "api/renders/:renderId/download",
    "routes/api.renders.$renderId.download.tsx",
  ),
  route("app", "routes/app.tsx", [
    index("routes/app._index.tsx"),
    route("print", "routes/app.print.tsx"),
    route("templates", "routes/app.templates.tsx"),
    route("templates/new", "routes/app.template-new.tsx"),
    route("templates/:templateId", "routes/app.templates.$templateId.tsx"),
    route("automations", "routes/app.automations.tsx"),
    route("activity", "routes/app.activity.tsx"),
    route("settings", "routes/app.settings.tsx"),
    route("billing", "routes/app.billing.tsx"),
    route("billing/confirm", "routes/app.billing-confirm.tsx"),
  ]),
] satisfies RouteConfig;
