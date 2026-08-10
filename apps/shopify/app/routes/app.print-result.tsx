export default function PrintResult() {
  return (
    <s-page heading="Printing 12 documents">
      <s-button slot="secondary-actions" href="/app/activity">
        View activity
      </s-button>
      <s-section>
        <s-stack direction="block" gap="base">
          <div className="piqae-card">
            <s-stack direction="block" gap="base">
              <div className="piqae-actions">
                <s-spinner accessibilityLabel="Printing documents" />
                <s-heading>Sending to Warehouse printer</s-heading>
              </div>
              <progress
                className="piqae-progress"
                max="12"
                value="8"
                aria-label="8 of 12 documents submitted"
              >
                8 of 12
              </progress>
              <s-paragraph>8 submitted · 4 preparing</s-paragraph>
              <s-paragraph>
                You can leave this page. Printing continues in the background
                and the result will appear in Activity.
              </s-paragraph>
            </s-stack>
          </div>
          <s-banner tone="info">
            Submitted means the printer spooler accepted the job. It does not
            confirm ink reached paper.
          </s-banner>
          <div className="piqae-actions">
            <s-button href="/app/print">Print more orders</s-button>
            <s-button>Download combined PDF</s-button>
          </div>
        </s-stack>
      </s-section>
    </s-page>
  );
}
