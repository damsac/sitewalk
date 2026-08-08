import Foundation

/// The trades an operator can pick during onboarding.
///
/// Isaac, 2026-08-08: *"If someone who doesn't belong to one of the trades
/// listed gets the app, I don't want them to think this isn't for them."*
///
/// The picker used to offer exactly three — landscape, property, inspection —
/// which told a plumber, a roofer, or a handyman that the app was built for
/// somebody else. That is an expensive thing to say on the first screen, and it
/// was never true: nothing about walking a job and talking is landscaping-
/// specific.
///
/// ## What the trade key actually does
///
/// Less than it looks. It picks the starter vocabulary pack (only three ship,
/// and a trade without one simply seeds nothing), it scopes document types the
/// operator authors themselves, and it tags the walk's template in core. It
/// does NOT gate the general document types — Estimate, Invoice, Work Order and
/// Report are universal, so every trade below can produce all four.
///
/// ## Why the keys must never change
///
/// A key is persisted in `BusinessProfile` AND written to `document_schemas.
/// trade_key` in core. Renaming one orphans the operator's own document types.
/// Add freely; never rename or remove.
enum Trades {
    struct Option: Identifiable, Hashable {
        let key: String
        let label: String
        /// The document this trade most often needs. Sets expectations in the
        /// picker without promising anything the app can't do.
        let stamp: String
        var id: String { key }
    }

    /// The key stored when an operator picks "Something else". Their typed
    /// answer is kept as the business's own trade label; the key stays stable so
    /// their schemas survive.
    static let otherKey = "other"

    /// Ordered roughly by how many US businesses each represents, with the three
    /// that ship vocabulary packs first.
    static let catalog: [Option] = [
        Option(key: "landscape",   label: "Landscaping & lawn",    stamp: "ESTIMATES"),
        Option(key: "property",    label: "Property mgmt",         stamp: "MOVE-OUT REPORTS"),
        Option(key: "inspection",  label: "Home inspection",       stamp: "INSPECTION REPORTS"),
        Option(key: "handyman",    label: "Handyman",              stamp: "ESTIMATES"),
        Option(key: "general",     label: "General contracting",   stamp: "ESTIMATES"),
        Option(key: "remodel",     label: "Remodeling",            stamp: "ESTIMATES"),
        Option(key: "plumbing",    label: "Plumbing",              stamp: "INVOICES"),
        Option(key: "hvac",        label: "HVAC",                  stamp: "WORK ORDERS"),
        Option(key: "electrical",  label: "Electrical",            stamp: "INVOICES"),
        Option(key: "roofing",     label: "Roofing",               stamp: "ESTIMATES"),
        Option(key: "painting",    label: "Painting",              stamp: "ESTIMATES"),
        Option(key: "tree",        label: "Tree service",          stamp: "ESTIMATES"),
        Option(key: "pest",        label: "Pest control",          stamp: "WORK ORDERS"),
        Option(key: "cleaning",    label: "Cleaning",              stamp: "ESTIMATES"),
        Option(key: "restoration", label: "Restoration",           stamp: "REPORTS"),
        Option(key: "pool",        label: "Pool & spa",            stamp: "WORK ORDERS"),
        Option(key: "concrete",    label: "Concrete & masonry",    stamp: "ESTIMATES"),
        Option(key: "fencing",     label: "Fencing",               stamp: "ESTIMATES"),
        Option(key: otherKey,      label: "Something else",        stamp: "ESTIMATES"),
    ]

    /// The display label for a stored key. Falls back to the key itself so a
    /// value written by a future build never renders blank.
    static func label(for key: String) -> String {
        catalog.first { $0.key == key }?.label ?? key.capitalized
    }

    /// Whether a key is one the app ships content for. Used to decide whether to
    /// ask the "what do you do?" follow-up.
    static func isKnown(_ key: String) -> Bool {
        catalog.contains { $0.key == key }
    }
}
