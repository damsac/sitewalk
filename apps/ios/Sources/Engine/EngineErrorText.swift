import Foundation

/// Turns an engine error into something a contractor can read.
///
/// UniFFI conforms its generated error enums to `LocalizedError` with
/// `errorDescription = String(reflecting: self)`, so `localizedDescription`
/// hands back the Swift *reflection* of the case:
///
/// ```
/// MurmurCoreFFI.EngineError.Document(message: "document build error: invalid
/// state: 'prf' is not a legal document kind for template Some(\"landscape\")")
/// ```
///
/// That shipped to TestFlight and Isaac photographed it (2026-07-28). The
/// payload inside it was genuinely useful — it named a real bug — but the
/// wrapper is Swift-module trivia, the `\"` escapes are an artifact of being
/// printed twice, and none of it belongs on a job site. App Review would see
/// the same thing.
///
/// So: keep the message, drop the packaging. This deliberately does NOT
/// rewrite errors into friendly copy — a vague "something went wrong" is what
/// made the previous version of this screen useless, and the specific text is
/// what turns a field report into a fix. It only removes what carries no
/// information.
enum EngineErrorText {
    /// Rust error-variant names that lead the payload. They name a `CoreError`
    /// case, which is an implementation detail of the crate — the sentence
    /// after them is the part with meaning.
    private static let noisePrefixes = ["invalid state: ", "invalid input: "]

    static func readable(_ error: Error) -> String {
        let raw = error.localizedDescription
        var text = payload(in: raw) ?? raw

        // Strip variant names wherever they lead a clause. "document build
        // error: invalid state: 'prf' is not…" keeps the useful "document build
        // error:" context and loses the enum name in the middle.
        for prefix in noisePrefixes {
            text = text.replacingOccurrences(of: prefix, with: "")
        }
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Extracts the `message: "…"` payload from UniFFI's reflected description,
    /// or nil when the shape doesn't match — a StoreKit error, an `NSError`, or
    /// a future UniFFI that formats differently all fall through to the raw
    /// description rather than being mangled.
    private static func payload(in raw: String) -> String? {
        guard let start = raw.range(of: "(message: \""),
              raw.hasSuffix("\")")
        else { return nil }
        let inner = raw[start.upperBound..<raw.index(raw.endIndex, offsetBy: -2)]
        guard !inner.isEmpty else { return nil }
        // Undo one level of escaping: the payload was rendered as a Swift
        // string literal, so its own quotes arrived as \" and its backslashes
        // as \\. Order matters — quotes first, then backslashes, or a literal
        // \\" would collapse wrongly.
        return String(inner)
            .replacingOccurrences(of: "\\\"", with: "\"")
            .replacingOccurrences(of: "\\\\", with: "\\")
    }
}
