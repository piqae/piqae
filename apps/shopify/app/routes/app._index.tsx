import type { LoaderFunctionArgs } from "react-router";
import { redirect } from "react-router";

export function templatesIndexUrl(requestUrl: string): string {
  return `/app/templates${new URL(requestUrl).search}`;
}

export function loader({ request }: LoaderFunctionArgs) {
  return redirect(templatesIndexUrl(request.url), 302);
}

export default function AppIndex() {
  return null;
}
