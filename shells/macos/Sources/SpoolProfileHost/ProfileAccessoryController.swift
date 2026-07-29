import AppKit

@MainActor
final class ProfileAccessoryController: NSViewController, NSPrintPanelAccessorizing {
    private let nameField = NSTextField()
    private let stockField = NSTextField()
    private let copiesCheckbox = NSButton(checkboxWithTitle: "Copies", target: nil, action: nil)
    private let pagesCheckbox = NSButton(checkboxWithTitle: "Page range", target: nil, action: nil)

    init(profileName: String, stockID: String? = nil, safeOverrides: [String] = ["copies"]) {
        super.init(nibName: nil, bundle: nil)
        nameField.stringValue = profileName
        stockField.stringValue = stockID ?? ""
        copiesCheckbox.state = safeOverrides.contains("copies") ? .on : .off
        pagesCheckbox.state = safeOverrides.contains("pages") ? .on : .off
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override var title: String? {
        get { "Spool Profile" }
        set {}
    }

    var profileName: String {
        nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var stockID: String? {
        let value = stockField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        return value.isEmpty ? nil : value
    }

    var safeOverrides: [String] {
        var result: [String] = []
        if copiesCheckbox.state == .on { result.append("copies") }
        if pagesCheckbox.state == .on { result.append("pages") }
        return result
    }

    override func loadView() {
        nameField.placeholderString = "A4 colour, Tray 1"
        nameField.setAccessibilityLabel("Profile name")
        stockField.placeholderString = "Optional stock ID"
        stockField.setAccessibilityLabel("Stock ID")

        let nameRow = labelledRow(label: "Profile name:", control: nameField)
        let stockRow = labelledRow(label: "Stock:", control: stockField)
        let overridesLabel = NSTextField(labelWithString: "API overrides:")
        overridesLabel.alignment = .right
        overridesLabel.setContentHuggingPriority(.required, for: .horizontal)
        let overrides = NSStackView(views: [copiesCheckbox, pagesCheckbox])
        overrides.orientation = .horizontal
        overrides.spacing = 12
        let overridesRow = NSStackView(views: [overridesLabel, overrides])
        overridesRow.orientation = .horizontal
        overridesRow.alignment = .centerY
        overridesRow.spacing = 8

        let note = NSTextField(
            wrappingLabelWithString:
                "The printer driver controls paper, tray, colour, quality, and vendor settings. "
                + "Spool saves them without printing."
        )
        note.textColor = .secondaryLabelColor
        note.maximumNumberOfLines = 3

        let stack = NSStackView(views: [nameRow, stockRow, overridesRow, note])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 12, right: 12)
        stack.translatesAutoresizingMaskIntoConstraints = false

        let container = NSView()
        container.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            stack.topAnchor.constraint(equalTo: container.topAnchor),
            stack.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            nameField.widthAnchor.constraint(greaterThanOrEqualToConstant: 260),
            stockField.widthAnchor.constraint(greaterThanOrEqualToConstant: 260),
        ])
        view = container
    }

    func localizedSummaryItems() -> [[NSPrintPanel.AccessorySummaryKey: String]] {
        [
            [
                .itemName: "Spool profile",
                .itemDescription: profileName.isEmpty ? "Unnamed" : profileName,
            ],
            [
                .itemName: "Stock",
                .itemDescription: stockID ?? "Not assigned",
            ],
            [
                .itemName: "Safe API overrides",
                .itemDescription: safeOverrides.isEmpty
                    ? "None"
                    : safeOverrides.joined(separator: ", "),
            ],
        ]
    }

    private func labelledRow(label: String, control: NSView) -> NSStackView {
        let labelView = NSTextField(labelWithString: label)
        labelView.alignment = .right
        labelView.setContentHuggingPriority(.required, for: .horizontal)
        let row = NSStackView(views: [labelView, control])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 8
        return row
    }
}
