import type { LoaderFunctionArgs } from "react-router";
import { Outlet, useLoaderData } from "react-router";
import { AppProvider } from "@shopify/shopify-app-react-router/react";
import shopify from "../shopify.server";

export async function loader({ request }: LoaderFunctionArgs) {
  await shopify.authenticate.admin(request);
  return { apiKey: process.env.SHOPIFY_API_KEY ?? "" };
}
export default function App() {
  const { apiKey } = useLoaderData<typeof loader>();
  return (
    <AppProvider embedded apiKey={apiKey}>
      <ui-nav-menu>
        <a href="/app/templates">Templates</a>
        <a href="/app/activity">Activity</a>
        <a href="/app/printers">Printers</a>
        <a href="/app/settings">Settings</a>
        <a href="/app/billing">Plan</a>
      </ui-nav-menu>
      <Outlet />
    </AppProvider>
  );
}
