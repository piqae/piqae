import type { LoaderFunctionArgs } from "react-router";

import { loadAdminPrintOptions } from "../core/admin-print-options.server";
import { workflows } from "../core/workflows.server";
import { createProductionServices } from "../services.server";
import shopify from "../shopify.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const { session, cors } = await shopify.authenticate.admin(request);
  const services = createProductionServices();
  try {
    await services.managedAccounts.ensure(session.shop);
    const result = await loadAdminPrintOptions({
      shop: session.shop,
      shops: services.repository,
      workflows: workflows(),
      vault: services.vault,
      baseUrl: services.baseUrl,
      appUrl: services.appUrl,
      managedClientFactory: (link) => services.managedAccounts.client(link),
    });
    return cors(Response.json(result));
  } catch (error) {
    const message = error instanceof Error ? error.message : "unknown error";
    return cors(
      Response.json(
        {
          error: "Printing destinations could not be loaded",
          detail: process.env.NODE_ENV === "development" ? message : undefined,
        },
        { status: 502 },
      ),
    );
  }
}
