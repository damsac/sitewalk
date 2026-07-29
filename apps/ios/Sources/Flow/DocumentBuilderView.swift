import SwiftUI

// The Document Builder — the STRUCTURE half of customizable paperwork, on top
// of dam's `DocumentSchema` seam (#244). Operators reorder and rename sections,
// add fields, and spin up document types we don't ship; the walk then fills
// whatever they authored.
//
// Two things worth knowing about this screen's role:
//
// 1. It is ALSO the confirm step for "upload your own template". Inference
//    produces a DRAFT schema and hands it here pre-populated instead of empty;
//    the operator confirms, and only then is it saved. That is what keeps
//    upload from being "trust the model to read a stranger's document on every
//    walk" — the comprehension pass runs once, under human review. So this is
//    built once and serves both paths.
//
// 2. Core is the authority on what's legal. `save_document_schema` runs
//    `validate_schema`, which REJECTS rather than coerces (R6). The editor
//    only ever offers allowlisted kinds and gates Save on `SchemaValidation`,
//    so that rejection is normally unreachable — but when it does fire, the
//    message is surfaced verbatim rather than swallowed.
//
// Reached from the board header, same sheet pattern as the Letterhead Studio.
struct DocumentBuilderView: View {
    @Bindable var model: AppModel
    /// Drop this screen's own header when hosted inside the My Business sheet.
    let embedded: Bool
    @Environment(\.dismiss) private var dismiss

    @State private var schemas: [DocumentSchemaModel] = []
    @State private var editing: DocumentSchemaModel?
    @State private var loadError: String?

    init(model: AppModel, embedded: Bool = false) {
        self.model = model
        self.embedded = embedded
    }

    var body: some View {
        VStack(spacing: 0) {
            if !embedded { header }
            if let editing {
                SchemaEditor(
                    schema: editing,
                    tradeKey: model.trade.key,
                    onCancel: { self.editing = nil },
                    onSave: { save($0) },
                    onDelete: editing.isBuiltin || editing.id.isEmpty
                        ? nil : { delete(editing) }
                )
            } else {
                list
            }
        }
        .background(Theme.C.paper)
        .onAppear(perform: load)
    }

    // MARK: Header

    private var header: some View {
        HStack {
            Text("DOCUMENT BUILDER")
                .font(Theme.F.mono(11, .semibold))
                .tracking(1.4)
                .foregroundStyle(Theme.C.ink60)
            Spacer()
            Button("CLOSE") { dismiss() }
                .font(Theme.F.mono(11, .semibold))
                .tracking(1.0)
                .foregroundStyle(Theme.C.ink60)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.C.hairline).frame(height: 1)
        }
    }

    // MARK: Doc-type list

    private var list: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text("Your walks fill these documents. Duplicate one to change what's in it.")
                    .font(Theme.F.ui(14, .regular))
                    .foregroundStyle(Theme.C.ink60)
                    .padding(.horizontal, 20)
                    .padding(.top, 16)
                    .padding(.bottom, 18)

                if let loadError {
                    ErrorNote(text: loadError)
                        .padding(.horizontal, 20)
                        .padding(.bottom, 12)
                }

                ForEach(schemas) { schema in
                    Button { editing = schema } label: {
                        SchemaRow(schema: schema)
                    }
                    .buttonStyle(.plain)
                }

                Button {
                    editing = DocumentSchemaModel(
                        kind: "",
                        label: "",
                        numberPrefix: "",
                        tradeKey: model.trade.key,
                        sections: [Self.starterLineItems()]
                    )
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "plus")
                        Text("NEW DOCUMENT TYPE")
                            .font(Theme.F.mono(11, .semibold))
                            .tracking(1.2)
                    }
                    .foregroundStyle(Theme.C.orangeDeep)
                    .padding(.horizontal, 20)
                    .padding(.vertical, 18)
                }
                .buttonStyle(.plain)
            }
        }
    }

    /// A new type starts WITH a line-items section, because core requires
    /// exactly one and an operator has no way to know that. Starting empty
    /// would mean every new document type opens in an invalid state.
    private static func starterLineItems() -> SchemaSectionModel {
        SchemaSectionModel(
            key: "line_items",
            kind: "line_items",
            label: "Items",
            priced: true,
            fields: [
                SchemaFieldModel(
                    key: "items", kind: "line_items", label: "Captured items", fill: "walk"
                )
            ]
        )
    }

    // MARK: Engine round-trips

    private func load() {
        do {
            schemas = try model.engine.listDocumentSchemas(tradeKey: model.trade.key)
            loadError = nil
        } catch {
            // Leave whatever is on screen intact rather than blanking the list
            // (the vocabulary editor's posture).
            loadError = "Couldn't load document types: \(error.localizedDescription)"
        }
    }

    private func save(_ schema: DocumentSchemaModel) {
        do {
            _ = try model.engine.saveDocumentSchema(schema)
            editing = nil
            load()
        } catch {
            loadError = "Couldn't save: \(error.localizedDescription)"
            editing = nil
        }
    }

    private func delete(_ schema: DocumentSchemaModel) {
        do {
            try model.engine.removeDocumentSchema(id: schema.id)
            editing = nil
            load()
        } catch {
            loadError = "Couldn't delete: \(error.localizedDescription)"
            editing = nil
        }
    }
}

// MARK: - Row

private struct SchemaRow: View {
    let schema: DocumentSchemaModel

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(schema.label.isEmpty ? "Untitled" : schema.label)
                    .font(Theme.F.serif(17, .semibold))
                    .foregroundStyle(Theme.C.ink)
                Text(summary)
                    .font(Theme.F.mono(10, .regular))
                    .tracking(0.6)
                    .foregroundStyle(Theme.C.ink35)
            }
            Spacer()
            if schema.isBuiltin {
                Text("DEFAULT")
                    .font(Theme.F.mono(8, .semibold))
                    .tracking(1.0)
                    .foregroundStyle(Theme.C.ink60)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .background(Theme.C.paperDeep)
            }
            Image(systemName: "chevron.right")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(Theme.C.ink35)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
        .contentShape(Rectangle())
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.C.hairlineSoft).frame(height: 1)
        }
    }

    private var summary: String {
        let sections = schema.sections.count
        let fields = schema.sections.reduce(0) { $0 + $1.fields.count }
        return "\(schema.numberPrefix)-0000  ·  \(sections) SECTIONS  ·  \(fields) FIELDS"
    }
}

private struct ErrorNote: View {
    let text: String
    var body: some View {
        Text(text)
            .font(Theme.F.ui(13, .regular))
            .foregroundStyle(Theme.C.redTag)
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.C.redTint)
    }
}

// MARK: - Editor

private struct SchemaEditor: View {
    @State private var draft: DocumentSchemaModel
    private let original: DocumentSchemaModel
    let onCancel: () -> Void
    let onSave: (DocumentSchemaModel) -> Void
    /// nil for built-ins and unsaved drafts — neither can be deleted.
    let onDelete: (() -> Void)?
    /// The operator's trade, stamped onto a copy of a built-in. See `saveShape`.
    private let tradeKey: String?

    init(
        schema: DocumentSchemaModel,
        tradeKey: String?,
        onCancel: @escaping () -> Void,
        onSave: @escaping (DocumentSchemaModel) -> Void,
        onDelete: (() -> Void)?
    ) {
        self.original = schema
        self.tradeKey = tradeKey
        _draft = State(initialValue: schema)
        self.onCancel = onCancel
        self.onSave = onSave
        self.onDelete = onDelete
    }

    private var problem: String? { SchemaValidation.firstProblem(in: draft) }

    /// What actually gets sent to `save_document_schema`.
    ///
    /// `save` upserts BY ID, so sending a built-in's id back overwrites the
    /// built-in in place — while this screen promised "saving creates your own
    /// copy, the default stays available." That was simply false, and it took
    /// the shipped default with it.
    ///
    /// Clearing the id makes core mint a new one, so the copy is real. If the
    /// operator also renamed it, the kind is re-derived: a built-in keeps its
    /// kind frozen (that is what `buildDocument` resolves against), but a
    /// RENAMED copy is a different document type and must not keep answering
    /// to "estimate" — otherwise a button labelled RFP builds an estimate.
    private func saveShape(of draft: DocumentSchemaModel) -> DocumentSchemaModel {
        guard draft.isBuiltin else { return draft }
        var copy = draft
        copy.id = ""            // create, don't overwrite the shipped default
        copy.isBuiltin = false
        // Stamp the operator's trade. The shipped `report` built-in carries
        // `trade_key: nil`, and core only resolves a nil-trade schema for a
        // nil-template session — so copying the nil through produced a document
        // type that appeared in every picker and built under none of them
        // (Isaac, TestFlight 2026-07-28: "'prf' is not a legal document kind
        // for template Some(\"landscape\")").
        //
        // A copy belongs to the operator who made it, so their trade is the
        // right answer regardless. For an already trade-scoped built-in this is
        // a no-op; for the universal one it's what makes duplicating Report the
        // way a landscaper gets a working Report.
        copy.tradeKey = tradeKey
        if draft.label != original.label {
            copy.kind = Self.slug(draft.label)
        }
        return copy
    }

    var body: some View {
        VStack(spacing: 0) {
            editorBar
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    if draft.isBuiltin { builtinNote }
                    identity
                    sections
                    addSectionButton
                    if let onDelete, !draft.isBuiltin { deleteButton(onDelete) }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 18)
            }
        }
    }

    private var editorBar: some View {
        HStack {
            Button("CANCEL", action: onCancel)
                .font(Theme.F.mono(11, .semibold))
                .foregroundStyle(Theme.C.ink60)
            Spacer()
            Button("SAVE") { onSave(saveShape(of: draft)) }
                .font(Theme.F.mono(11, .semibold))
                .foregroundStyle(problem == nil ? Theme.C.orangeDeep : Theme.C.ink35)
                .disabled(problem != nil)
        }
        .tracking(1.0)
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.C.hairline).frame(height: 1)
        }
        .overlay(alignment: .bottomLeading) {
            // Say WHY Save is unavailable. A disabled button with no reason is
            // the most common way an editor like this becomes unusable.
            if let problem {
                Text(problem)
                    .font(Theme.F.ui(12, .regular))
                    .foregroundStyle(Theme.C.ink60)
                    .padding(.horizontal, 20)
                    .offset(y: 18)
            }
        }
        .padding(.bottom, problem == nil ? 0 : 18)
    }

    private var builtinNote: some View {
        Text(
            "This is a default document type. Saving makes your own copy — the "
                + "original stays. Rename it and it becomes a new kind of document."
        )
        .font(Theme.F.ui(13, .regular))
        .foregroundStyle(Theme.C.ink60)
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.C.orangeTint)
    }

    private var identity: some View {
        VStack(alignment: .leading, spacing: 12) {
            FieldLabel("DOCUMENT TYPE")
            LabeledField(title: "Name", text: $draft.label, placeholder: "Estimate")
                .onChange(of: draft.label) { _, new in
                    // Derive the stable key from the name until the operator is
                    // saved once. After that the key is frozen: it is what core
                    // resolves `buildDocument(kind:)` against, so renaming a
                    // saved type must not silently orphan its documents.
                    if original.id.isEmpty { draft.kind = Self.slug(new) }
                }
            LabeledField(
                title: "Number prefix", text: $draft.numberPrefix, placeholder: "EST"
            )
            Text("Documents will be numbered \(prefixPreview)-0001, -0002, and so on.")
                .font(Theme.F.mono(10, .regular))
                .foregroundStyle(Theme.C.ink35)
        }
    }

    private var prefixPreview: String {
        let p = draft.numberPrefix.trimmingCharacters(in: .whitespaces).uppercased()
        return p.isEmpty ? "EST" : p
    }

    private static func slug(_ s: String) -> String {
        s.lowercased()
            .map { $0.isLetter || $0.isNumber ? String($0) : "_" }
            .joined()
            .split(separator: "_", omittingEmptySubsequences: true)
            .joined(separator: "_")
    }

    private var sections: some View {
        VStack(alignment: .leading, spacing: 14) {
            FieldLabel("SECTIONS")
            ForEach($draft.sections) { $section in
                SectionCard(
                    section: $section,
                    canDelete: section.kind != "line_items",
                    onDelete: { draft.sections.removeAll { $0.id == section.id } },
                    onMoveUp: { move(section.id, by: -1) },
                    onMoveDown: { move(section.id, by: 1) }
                )
            }
        }
    }

    private func move(_ id: UUID, by offset: Int) {
        guard let i = draft.sections.firstIndex(where: { $0.id == id }) else { return }
        let target = i + offset
        guard draft.sections.indices.contains(target) else { return }
        draft.sections.swapAt(i, target)
    }

    private var addSectionButton: some View {
        Button {
            draft.sections.append(
                SchemaSectionModel(
                    key: "section_\(draft.sections.count + 1)",
                    kind: "filled",
                    label: "",
                    priced: false,
                    fields: []
                )
            )
        } label: {
            Label("ADD SECTION", systemImage: "plus")
                .font(Theme.F.mono(11, .semibold))
                .tracking(1.2)
                .foregroundStyle(Theme.C.orangeDeep)
        }
        .buttonStyle(.plain)
    }

    private func deleteButton(_ action: @escaping () -> Void) -> some View {
        Button("DELETE THIS DOCUMENT TYPE", action: action)
            .font(Theme.F.mono(11, .semibold))
            .tracking(1.0)
            .foregroundStyle(Theme.C.redTag)
            .padding(.top, 8)
    }
}

// MARK: - Section card

private struct SectionCard: View {
    @Binding var section: SchemaSectionModel
    let canDelete: Bool
    let onDelete: () -> Void
    let onMoveUp: () -> Void
    let onMoveDown: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                TextField("Section name", text: $section.label)
                    .font(Theme.F.serif(16, .semibold))
                    .foregroundStyle(Theme.C.ink)
                Spacer()
                Button(action: onMoveUp) { Image(systemName: "arrow.up") }
                Button(action: onMoveDown) { Image(systemName: "arrow.down") }
                if canDelete {
                    Button(action: onDelete) { Image(systemName: "trash") }
                        .foregroundStyle(Theme.C.redTag)
                }
            }
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(Theme.C.ink60)

            if section.kind == "line_items" {
                // The one section every document must have, and the only one
                // whose contents aren't authored field-by-field: it's where the
                // walk's captured items land.
                Text("The items from your walk land here.")
                    .font(Theme.F.ui(13, .regular))
                    .foregroundStyle(Theme.C.ink60)
                Toggle("Show prices", isOn: $section.priced)
                    .font(Theme.F.ui(14, .regular))
                    .tint(Theme.C.orange)
            } else {
                ForEach($section.fields) { $field in
                    FieldCard(
                        field: $field,
                        onDelete: { section.fields.removeAll { $0.id == field.id } }
                    )
                }
                Button {
                    section.fields.append(
                        SchemaFieldModel(
                            key: "field_\(section.fields.count + 1)",
                            kind: "text",
                            label: "",
                            fill: "walk"
                        )
                    )
                } label: {
                    Label("Add field", systemImage: "plus")
                        .font(Theme.F.ui(13, .regular))
                        .foregroundStyle(Theme.C.orangeDeep)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(14)
        .background(Theme.C.sheet)
        .overlay(Rectangle().stroke(Theme.C.hairline, lineWidth: 1))
    }
}

// MARK: - Field card

private struct FieldCard: View {
    @Binding var field: SchemaFieldModel
    let onDelete: () -> Void

    /// Plain-language labels for the fill modes. "walk" / "manual" / "static"
    /// are core's vocabulary, not a contractor's.
    private static let fillLabels: [(value: String, label: String)] = [
        ("walk", "From the walk"),
        ("manual", "I fill it in"),
        ("static", "Always the same"),
    ]

    private static let kindLabels: [(value: String, label: String)] = [
        ("text", "Short text"),
        ("long_text", "Paragraph"),
        ("currency", "Amount"),
        ("quantity", "Quantity"),
        ("date", "Date"),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                TextField("Field name", text: $field.label)
                    .font(Theme.F.ui(14, .regular))
                Button(action: onDelete) { Image(systemName: "minus.circle") }
                    .foregroundStyle(Theme.C.ink35)
            }
            HStack(spacing: 8) {
                Picker("Type", selection: $field.kind) {
                    ForEach(Self.kindLabels, id: \.value) { Text($0.label).tag($0.value) }
                }
                Picker("Fill", selection: $field.fill) {
                    ForEach(Self.fillLabels, id: \.value) { Text($0.label).tag($0.value) }
                }
            }
            .pickerStyle(.menu)
            .font(Theme.F.mono(10, .regular))
            .tint(Theme.C.orangeDeep)

            if field.fill == "static" {
                TextField(
                    "Text that always appears",
                    text: Binding(
                        get: { field.staticValue ?? "" },
                        set: { field.staticValue = $0 }
                    )
                )
                .font(Theme.F.ui(13, .regular))
                .padding(8)
                .background(Theme.C.paperDeep)
            }
        }
        .padding(.vertical, 6)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.C.hairlineSoft).frame(height: 1)
        }
    }
}

// MARK: - Small shared bits

private struct FieldLabel: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text)
            .font(Theme.F.mono(10, .semibold))
            .tracking(1.4)
            .foregroundStyle(Theme.C.ink35)
    }
}

private struct LabeledField: View {
    let title: String
    @Binding var text: String
    let placeholder: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(Theme.F.ui(12, .regular))
                .foregroundStyle(Theme.C.ink60)
            TextField(placeholder, text: $text)
                .font(Theme.F.ui(16, .regular))
                .padding(10)
                .background(Theme.C.sheet)
                .overlay(Rectangle().stroke(Theme.C.hairline, lineWidth: 1))
        }
    }
}
