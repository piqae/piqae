import type { LoaderFunctionArgs } from "react-router";
import { Form, useLoaderData } from "react-router";
import shopify from "../shopify.server";
import { workflows } from "../core/workflows.server";
export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const url = new URL(request.url);
  const query = url.searchParams.get("query")?.slice(0, 100) ?? "";
  const state = url.searchParams.get("state") ?? "";
  return {
    entries: await workflows().listActivity(session.shop, query, state),
    query,
    state,
  };
}
export default function Activity() {
  const { entries, query, state } = useLoaderData<typeof loader>();
  return (
    <s-page heading="Print activity">
      <s-section>
        <s-stack direction="block" gap="base">
          <Form method="get" className="piqae-toolbar">
            <label>
              Search
              <input
                className="piqae-input"
                name="query"
                defaultValue={query}
                maxLength={100}
              />
            </label>
            <label>
              Status
              <select className="piqae-input" name="state" defaultValue={state}>
                <option value="">All statuses</option>
                <option value="accepted">Accepted</option>
                <option value="printing">Printing</option>
                <option value="reported_complete">Reported complete</option>
                <option value="uncertain">Uncertain</option>
                <option value="failed">Failed</option>
              </select>
            </label>
            <s-button type="submit">Filter</s-button>
          </Form>
          <div className="piqae-card">
            <table className="piqae-list">
              <thead>
                <tr>
                  <th>Order</th>
                  <th>Document</th>
                  <th>Destination</th>
                  <th>Status</th>
                  <th>Created</th>
                </tr>
              </thead>
              <tbody>
                {entries.map((entry) => (
                  <tr key={entry.id}>
                    <td data-label="Order">{entry.orderName}</td>
                    <td data-label="Document">{entry.documentName}</td>
                    <td data-label="Destination">{entry.destination}</td>
                    <td data-label="Status">
                      {entry.state.replaceAll("_", " ")}
                    </td>
                    <td data-label="Created">
                      <time dateTime={entry.createdAt}>
                        {new Date(entry.createdAt).toLocaleString()}
                      </time>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {entries.length === 0 ? (
              <s-paragraph>No matching activity yet.</s-paragraph>
            ) : null}
          </div>
          <s-banner tone="warning">
            Accepted by a spooler does not prove physical delivery. Uncertain
            jobs remain visible for review.
          </s-banner>
        </s-stack>
      </s-section>
    </s-page>
  );
}
