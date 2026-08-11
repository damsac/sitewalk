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
    /// What this field should contain, in the model's terms. Not editable in
    /// v1 — carried so a round-trip through the editor cannot silently strip
    /// it off a built-in and leave the compose pass writing one-word answers.
    var hint: String?

    init(
        id: UUID = UUID(),
        key: String,
        kind: String,
        label: String,
        fill: String,
        staticValue: String? = nil,
        hint: String? = nil
    ) {
        self.id = id
        self.key = key
        self.kind = kind
        self.label = label
        self.fill = fill
        self.staticValue = staticValue
        self.hint = hint
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
    /// `line_items` only: "" | "inclusion" | "directive" | "observation".
    /// Not editable in v1 — carried for the same reason as `hint`.
    var lineDetail: String
    var fields: [SchemaFieldModel]

    init(
        id: UUID = UUID(),
        key: String,
        kind: String,
        label: String,
        priced: Bool,
        lineDetail: String = "",
        fields: [SchemaFieldModel]
    ) {
        self.id = id
        self.key = key
        self.kind = kind
        self.label = label
        self.priced = priced
        self.lineDetail = lineDetail
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
    /// Which trade this belongs to.
    ///
    /// **`nil` means UNIVERSAL — available on every trade.** Core's
    /// `resolve_active_schema` matches `trade_key = ?template OR trade_key IS
    /// NULL`, preferring a trade-specific row when one exists. Filter with
    /// `buildable(from:tradeKey:)`, which mirrors that ordering.
    var tradeKey: String?
    var totalKind: String
    var totalLabelKey: String
    var sections: [SchemaSectionModel]
    var schemaVersion: UInt32
    /// Core stamps this on every save. Needed app-side because core resolves
    /// `buildDocument(kind:)` with `ORDER BY updated_at DESC LIMIT 1` — so when
    /// two schemas share a kind, this is what decides which one actually gets
    /// built, and the UI has to agree or it lies about what a button will do.
    var updatedAt: UInt64

    /// Built-ins ship seeded from core and carry the sentinel device id.
    /// Surfaced so the editor can steer toward "duplicate" rather than
    /// in-place edits of a shared default.
    var isBuiltin: Bool

    /// The schemas a walk on `tradeKey` can ACTUALLY build — a mirror of core's
    /// `resolve_active_schema`, so a button never offers a document that will
    /// then be refused.
    ///
    /// Two rules, both copied from the resolver rather than invented here:
    ///
    /// 1. **Trade matches, or the schema is UNIVERSAL.** A nil `tradeKey` means
    ///    "any trade" — Isaac's call, 2026-07-30: *"They should come in
    ///    regardless of trade!"* A type the operator authored belongs to them,
    ///    not to whichever trade they were in when they made it. Core's
    ///    resolver was changed to match, so a universal schema is now genuinely
    ///    buildable rather than merely listable.
    /// 2. **One winner per kind**, the newest. Core resolves with
    ///    `ORDER BY updated_at DESC LIMIT 1`, so when two schemas share a kind
    ///    exactly one is ever built; showing both would put a dead button on
    ///    screen and collide as `ForEach` ids, which is undefined in SwiftUI and
    ///    can swallow taps.
    ///
    /// Sorted by label for a stable order.
    static func buildable(
        from schemas: [DocumentSchemaModel], tradeKey: String?
    ) -> [DocumentSchemaModel] {
        let matching = schemas.filter { $0.tradeKey == tradeKey || $0.tradeKey == nil }
        return Dictionary(grouping: matching, by: \.kind)
            .compactMap { _, group in
                // The winner is the FIRST row under core's ORDER BY, term
                // for term:
                //
                //   (trade_key IS NULL) ASC — a trade-specific schema beats a
                //     universal one for the same kind whatever the timestamps
                //     say, so an operator's own version is never shadowed by a
                //     shared default. (Getting this backwards made the shared
                //     default win — caught by
                //     `testATradeSpecificSchemaBeatsAUniversalOneOfTheSameKind`.)
                //   updated_at DESC — newest breaks the remaining tie.
                //   id ASC — closes the order. Without it a tie (two universal
                //     rows, same `updatedAt` — the state a device is in the
                //     moment it duplicates a built-in and has not edited the
                //     copy) is broken by whichever order the rows happened to
                //     arrive in, and the mirror can disagree with the resolver
                //     on the same data.
                group.min { a, b in
                    if (a.tradeKey == nil) != (b.tradeKey == nil) { return b.tradeKey == nil }
                    if a.updatedAt != b.updatedAt { return a.updatedAt > b.updatedAt }
                    return a.id < b.id
                }
            }
            .sorted { $0.label < $1.label }
    }

    /// The stable `kind` derived from an operator-typed name: lowercased, every
    /// run of non-alphanumerics collapsed to one underscore, no leading or
    /// trailing underscore.
    static func slug(_ s: String) -> String {
        s.lowercased()
            .map { $0.isLetter || $0.isNumber ? String($0) : "_" }
            .joined()
            .split(separator: "_", omittingEmptySubsequences: true)
            .joined(separator: "_")
    }

    /// What the Document Builder actually sends to `save_document_schema` when
    /// the operator saves `draft`, which they opened from `original`.
    ///
    /// Only a BUILT-IN is reshaped; an operator's own type saves as-is.
    ///
    /// `save` upserts BY ID, so sending a built-in's id back overwrites the
    /// built-in in place — while the screen promised "saving creates your own
    /// copy, the default stays available." That was simply false, and it took
    /// the shipped default with it. Clearing the id makes core mint a new one,
    /// so the copy is real.
    ///
    /// Two things deliberately survive the copy:
    ///
    /// 1. **The kind, unless the operator renamed it.** A built-in keeps its
    ///    kind frozen (that is what `buildDocument` resolves against), but a
    ///    RENAMED copy is a different document type and must not keep answering
    ///    to "estimate" — otherwise a button labelled RFP builds an estimate.
    /// 2. **The source's trade.** #283 stamped the operator's current trade
    ///    here, to work around a nil-trade copy being unbuildable. Core now
    ///    treats nil as universal (#306), so the workaround is not only
    ///    unnecessary but backwards: stamping would pin a copy of the universal
    ///    Report to one trade, and Isaac's call (2026-07-30) is the opposite —
    ///    *"They should come in regardless of trade!"* A duplicate of Report
    ///    stays universal; a duplicate of the landscape Estimate stays
    ///    landscape. The copy keeps whatever scope the thing it came from had.
    static func saveShape(
        of draft: DocumentSchemaModel, editedFrom original: DocumentSchemaModel
    ) -> DocumentSchemaModel {
        guard draft.isBuiltin else { return draft }
        var copy = draft
        copy.id = ""            // create, don't overwrite the shipped default
        copy.isBuiltin = false
        if draft.label != original.label {
            copy.kind = slug(draft.label)
        }
        return copy
    }

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
        updatedAt: UInt64 = 0,
        isBuiltin: Bool = false
    ) {
        self.updatedAt = updatedAt
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
