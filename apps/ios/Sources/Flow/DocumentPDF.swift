import SwiftUI
import UIKit

// One schema, many renderings: the PDF is drawn from the same components as
// the on-screen sheet. US Letter, single page. The PDF is never the source
// of truth — it's an export of the document data.

enum DocumentPDF {

    /// The section with its unfilled fields dropped. See the call site: a gap
    /// is a message to the operator, and the operator is not who reads this.
    static func printableSection(_ section: DocSectionFixture) -> DocSectionFixture {
        var out = section
        // Anything with no value comes off, whether it is a gap the walk left
        // or an optional the operator chose not to fill. "TAP TO ADD" is an
        // instruction to the person holding the phone.
        // `deposit_held` is typed in a section (that is how the operator
        // enters it) but PRINTS in the total block, where the arithmetic that
        // uses it lives. Left in both, the page states the same number twice
        // and the second one looks like a different fact.
        // Money fields that PRINT in the total block are typed in a section
        // (that is how the operator enters them) and must not also appear
        // above the lines, or the page states the same number twice and the
        // second one reads as a different fact. Dates and windows are NOT in
        // this list: "Valid until" and "Due" are document facts, not
        // arithmetic, and they belong on the page where they are read.
        let printsInTotals: Set<String> = ["deposit_held", "tax"]
        out.fields = section.fields.filter {
            !($0.value ?? "").isEmpty && !printsInTotals.contains($0.key)
        }
        return out
    }

    /// The line as a CLIENT should receive it.
    ///
    /// Everything the app was saying to the operator comes off (Isaac,
    /// 2026-08-09: *"little tag lines like 'added by you' are not
    /// included"*). Concretely:
    ///
    /// - **Provenance and prompts** ("ADDED BY YOU", "NOT HEARD — TAP OR SAY
    ///   IT") go. They advertise which parts of the document the software got
    ///   wrong, which is nobody's business but the operator's — and "added by
    ///   you" on a line the client is being charged for actively invites the
    ///   question "so what did the app get wrong?"
    /// - **The price-book hint** ("↺ LAST 3: $110 · $120 · $125") goes. It is
    ///   what this operator charged their LAST THREE customers. Printing that
    ///   on a quote is a pricing leak.
    /// - **An unfilled amount prints blank**, not as a yellow dashed "——".
    ///   The operator chose to send it; the dash is warning styling, and a
    ///   client reading a warning colour on their own estimate learns only
    ///   that something is wrong.
    ///
    /// What SURVIVES is everything written for the reader: the title, the
    /// composed detail line, the quantity, real amounts, and the assignee —
    /// a work order's crew names are the whole point of the document.
    static func printableRow(_ row: DocRowFixture) -> DocRowFixture {
        var out = row
        if row.subWarn || OperatorNote.isOperatorFacing(row.sub) {
            out.sub = ""
        }
        out.subWarn = false
        out.hint = nil
        out.isEdit = false
        if row.isGap {
            out.amount = row.amount == "——" ? "" : row.amount
            out.qty = row.qty == "——" ? "" : row.qty
        }
        out.isGap = false
        return out
    }

    /// `biz`/`bizSub`/`docDate` default to the trade fixture — callers with a
    /// BusinessProfile pass the operator's letterhead (AppModel.makePDF); the
    /// gallery/demo path keeps rendering fixture paperwork unchanged.
    @MainActor
    static func render(
        trade: TradeFixture, document: DocumentModel,
        biz: String? = nil, bizSub: String? = nil, docDate: String? = nil,
        branding: Branding = .default, layout: DocumentLayout = .default
    ) -> URL? {
        let pageSize = CGSize(width: 612, height: 792) // US Letter @ 72 dpi

        let content = PDFPageView(
            trade: trade, document: document,
            biz: biz ?? trade.biz,
            bizSub: bizSub ?? trade.bizSub,
            docDate: docDate ?? trade.docDate,
            branding: branding, layout: layout
        )
        // Width is fixed; HEIGHT is whatever the document needs. It used to be
        // pinned to one page height, which did not fit long documents onto a
        // page — it CROPPED them, silently, with no error and nothing in the
        // suite watching. A twenty-line estimate is an ordinary walk, and the
        // lines that fell off were lines someone was being charged for.
        .frame(width: pageSize.width)
        .frame(minHeight: pageSize.height, alignment: .top)

        let renderer = ImageRenderer(content: content)
        renderer.scale = 3
        guard let image = renderer.uiImage else { return nil }

        // The document's own number names the file. It used to be the trade
        // FIXTURE's number, so every export — invoice, work order, report —
        // arrived in the client's inbox as "EST-0047.pdf".
        let fileName = document.docNumber.isEmpty ? trade.docNo : document.docNumber
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("\(fileName).pdf")
        let pdf = UIGraphicsPDFRenderer(bounds: CGRect(origin: .zero, size: pageSize))
        do {
            try pdf.writePDF(to: url) { ctx in
                // One tall render, sliced into pages: each page draws the whole
                // image shifted up by a page height, and the page bounds clip
                // the rest. Simple, and it cannot lose content the way a single
                // fixed-height page did.
                //
                // KNOWN LIMIT: a slice can land mid-row, so a line can be cut
                // across two pages. That is a cosmetic fault where cropping was
                // a correctness one, and the fix (laying out page by page,
                // measuring row heights) is a different piece of work. Recorded
                // rather than hidden — see the paperwork audit plan.
                let totalHeight = image.size.height
                let pageCount = max(1, Int(ceil(totalHeight / pageSize.height)))
                for page in 0..<pageCount {
                    ctx.beginPage()
                    image.draw(in: CGRect(
                        x: 0,
                        y: -CGFloat(page) * pageSize.height,
                        width: pageSize.width,
                        height: totalHeight
                    ))
                }
            }
            return url
        } catch {
            return nil
        }
    }
}

// Print layout: same letterhead + rows, paper margins, no app chrome.
private struct PDFPageView: View {
    let trade: TradeFixture
    let document: DocumentModel
    let biz: String
    let bizSub: String
    let docDate: String
    var branding: Branding = .default
    var layout: DocumentLayout = .default

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Letterhead(
                biz: biz,
                bizSub: bizSub,
                docKind: document.docKindLabel.isEmpty ? trade.docKind : document.docKindLabel,
                // The minted number, same as the preview — an exported PDF
                // that disagreed with the screen about its own document
                // number would be the worst possible place to disagree.
                docNo: document.docNumber.isEmpty ? trade.docNo : document.docNumber,
                docDate: docDate,
                branding: branding
            )
            // The authored blocks, above the lines — the same order as the
            // review screen, because "pixel-identical to the preview" is the
            // promise the whole review screen rests on.
            //
            // Gaps are the one deliberate difference: on screen an unfilled
            // field shows as "NOT HEARD — TAP OR SAY IT" so the operator can
            // fix it; in the exported PDF that sentence would be addressed to
            // the CLIENT, telling them the contractor's software missed
            // something. A block with nothing written in it is simply omitted.
            // Strip FIRST, then decide whether anything survived. Filtering on
            // `hasContent` before the strip let a section through whose only
            // field the strip then removed — printing "DEPOSIT" over nothing,
            // the same heading-over-an-empty-box defect the blocks and the
            // total row were already fixed for.
            ForEach(
                document.sections
                    .map(DocumentPDF.printableSection)
                    .filter(\.hasContent)
            ) { section in
                DocSectionView(section: section)
            }
            ForEach(document.rows) { row in
                DocRowView(row: DocumentPDF.printableRow(row))
            }
            // `gaps: 0` — the "+3 GAP" badge is the app nudging the operator
            // before they send. Once they have sent, it is a note to the
            // client that the estimate is incomplete.
            //
            // A document with nothing to total prints no total row — WO-0002
            // went to a crew with `TOTAL ——` under the assignment.
            if document.showsTotal {
                ForEach(document.totalLines) { line in
                    TotalRow(key: line.key, value: line.value, gaps: 0)
                        .padding(.top, line.strong ? 2 : 0)
                        .opacity(line.strong ? 1 : 0.85)
                }
            }
            // Structure basics (DocumentLayout): operator terms + signature line.
            if !layout.termsText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                TermsBlock(text: layout.termsText)
            }
            if layout.showSignature {
                SignatureRow()
            }
            Spacer(minLength: 0)
            // Free-tier footer ("PREPARED WITH JEFE"); nil once removed (Pro).
            if let footer = branding.footerText {
                Text(footer)
                    .font(Theme.F.mono(7))
                    .tracking(1.6)
                    .foregroundStyle(Theme.C.ink45)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.bottom, 8)
            }
        }
        .padding(48)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(Color.white)
    }

}


// MARK: - Share sheet wrapper

struct ShareSheet: UIViewControllerRepresentable {
    let url: URL
    /// `true` only when the user completed an activity; a cancelled sheet
    /// reports `false` and must not finalize the walk (issue #155).
    var onComplete: (Bool) -> Void

    func makeUIViewController(context: Context) -> UIActivityViewController {
        let vc = UIActivityViewController(activityItems: [url], applicationActivities: nil)
        vc.completionWithItemsHandler = { _, completed, _, _ in onComplete(completed) }
        return vc
    }

    func updateUIViewController(_ vc: UIActivityViewController, context: Context) {}
}
