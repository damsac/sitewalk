import Foundation

// App-side mirror of the core `DocumentSchema` seam (Plan 19, #244).
//
// Mirrored rather than used directly, for the same reason `DocumentModel` and
// `NotesModel` are: the demo engine has to work in a clean checkout where
// `#if canImport(MurmurCoreFFI)` is false and the FFI types don't exist. A view
// that referenced `MurmurCoreFFI.DocumentSchema` would compile only in
// real-core mode, which would make the Document Builder undemoable — exactly
// the surface most in need of design iteration without a Rust build.
//
// Conversion to/from the FFI records lives in `MurmurEngine`, the one file that
// is already FFI-conditional.

/// The kinds core accepts. Mirrors `VALID_*` in `murmur-core::domain`.
///
/// These are duplicated deliberately: `save_document_schema` validates against
/// the Rust allowlists and REJECTS anything else (R6, reject-never-coerce), so
/// the editor must only ever offer legal values. A picker that could author an
/// invalid kind would produce a save error the operator can't act on.
enum SchemaKinds {
    static let section = ["line_items", "static", "filled"]
    static let field = ["line_items", "text", "long_text", "currency", "quantity", "date", "static"]
    static let fill = ["walk", "manual", "static"]
}

struct SchemaFieldModel: Identifiable, Hashable {
    /// Stable identity for SwiftUI list moves/deletes. NOT the core `key` —
    /// the operator can rename a field, and a re-keyed row must not read as a
    /// different row mid-edit.
    let id: UUID
    var key: String
    /// One of `SchemaKinds.field`.
    var kind: String
    var label: String
    /// One of `SchemaKinds.fill`.
    var fill: String
    /// The authored constant when `fill == "static"`; nil otherwise.
    var staticValue: String?

    init(
        id: UUID = UUID(),
        key: String,
        kind: String,
        label: String,
        fill: String,
        staticValue: String? = nil
    ) {
        self.id = id
        self.key = key
        self.kind = kind
        self.label = label
        self.fill = fill
        self.staticValue = staticValue
    }
}

struct SchemaSectionModel: Identifiable, Hashable {
    let id: UUID
    var key: String
    /// One of `SchemaKinds.section`.
    var kind: String
    var label: String
    /// Whether the captured-items table carries amounts.
    var priced: Bool
    var fields: [SchemaFieldModel]

    init(
        id: UUID = UUID(),
        key: String,
        kind: String,
        label: String,
        priced: Bool,
        fields: [SchemaFieldModel]
    ) {
        self.id = id
        self.key = key
        self.kind = kind
        self.label = label
        self.priced = priced
        self.fields = fields
    }
}

struct DocumentSchemaModel: Identifiable, Hashable {
    /// Empty means "create" — core mints a UUIDv7. A built-in's fixed id or a
    /// prior save's id means upsert.
    var id: String
    /// Stable key: a built-in ("estimate") or a custom one ("hoa_addendum").
    var kind: String
    var label: String
    /// Core mints `<prefix>-NNNN`; the app never invents document numbers.
    var numberPrefix: String
    /// Which trade this belongs to; nil = all.
    var tradeKey: String?
    var totalKind: String
    var totalLabelKey: String
    var sections: [SchemaSectionModel]
    var schemaVersion: UInt32

    /// Built-ins ship seeded from core and carry the sentinel device id.
    /// Surfaced so the editor can steer toward "duplicate" rather than
    /// in-place edits of a shared default.
    var isBuiltin: Bool

    init(
        id: String = "",
        kind: String,
        label: String,
        numberPrefix: String,
        tradeKey: String? = nil,
        totalKind: String = "sum",
        totalLabelKey: String = "total",
        sections: [SchemaSectionModel] = [],
        schemaVersion: UInt32 = 1,
        isBuiltin: Bool = false
    ) {
        self.id = id
        self.kind = kind
        self.label = label
        self.numberPrefix = numberPrefix
        self.tradeKey = tradeKey
        self.totalKind = totalKind
        self.totalLabelKey = totalLabelKey
        self.sections = sections
        self.schemaVersion = schemaVersion
        self.isBuiltin = isBuiltin
    }
}

// MARK: - Validation

/// The one rule the editor must enforce before offering Save.
///
/// Core's `validate_schema` rejects a schema with 0 or 2+ `line_items`
/// sections, empty `kind`/`label`/`number_prefix`, or any kind outside the
/// allowlists. Checking here is not duplication for its own sake — it turns a
/// post-hoc save failure into a disabled button with a reason attached, which
/// is the difference between a usable editor and a guessing game. Core remains
/// the authority; this is a courtesy gate in front of it.
enum SchemaValidation {
    static func firstProblem(in schema: DocumentSchemaModel) -> String? {
        if schema.label.trimmingCharacters(in: .whitespaces).isEmpty {
            return "Give the document type a name."
        }
        if schema.kind.trimmingCharacters(in: .whitespaces).isEmpty {
            return "Give the document type an internal key."
        }
        let prefix = schema.numberPrefix.trimmingCharacters(in: .whitespaces)
        if prefix.isEmpty {
            return "Give the document type a number prefix, like EST."
        }
        let lineItemSections = schema.sections.filter { $0.kind == "line_items" }.count
        if lineItemSections == 0 {
            return "Add a line-items section — that's where the walk's items land."
        }
        if lineItemSections > 1 {
            return "Only one line-items section is supported."
        }
        for section in schema.sections {
            if section.label.trimmingCharacters(in: .whitespaces).isEmpty {
                return "Every section needs a name."
            }
            for field in section.fields
            where field.label.trimmingCharacters(in: .whitespaces).isEmpty {
                return "Every field in “\(section.label)” needs a name."
            }
        }
        return nil
    }
}
