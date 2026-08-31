import { redirect } from "react-router";

export function loader() {
  return redirect("/app/templates", 302);
}

export default function AppIndex() {
  return null;
}
