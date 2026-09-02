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
    const options = await loadAdminPrintOptions({
      shop: session.shop,
      shops: services.repository,
      workflows: workflows(),
      vault: services.vault,
      baseUrl: services.baseUrl,
      appUrl: services.appUrl,
      managedClientFactory: (link) => services.managedAccounts.client(link),
    });
    return cors(
      Response.json({
        ...options,
        documents: options.documents.filter(
          (document) => document.kind === "label",
        ),
      }),
    );
  } catch {
    return cors(
      Response.json(
        { error: "Product label destinations could not be loaded" },
        { status: 502 },
      ),
    );
  }
}
