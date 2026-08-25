import type { LoaderFunctionArgs } from "react-router";
import { redirect } from "react-router";
import shopify from "../shopify.server";

export async function loader({ request }: LoaderFunctionArgs) {
  await shopify.authenticate.admin(request);
  const url = new URL(request.url);
  const search = url.searchParams.toString();
  return redirect(`/app/printers${search ? `?${search}` : ""}`, 302);
}

export default function LegacySettingsRedirect() {
  return null;
}
