import SwiftUI
import UIKit

// One schema, many renderings: the PDF is drawn from the same components as
// the on-screen sheet. US Letter, single page. The PDF is never the source
// of truth — it's an export of the document data.

enum DocumentPDF {

    /// `biz`/`bizSub`/`docDate` default to the trade fixture — callers with a
    /// BusinessProfile pass the operator's letterhead (AppModel.makePDF); the
    /// gallery/demo path keeps rendering fixture paperwork unchanged.
    @MainActor
    static func render(
        trade: TradeFixture, document: DocumentModel,
        biz: String? = nil, bizSub: String? = nil, docDate: String? = nil,
        branding: Branding = .default
    ) -> URL? {
        let pageSize = CGSize(width: 612, height: 792) // US Letter @ 72 dpi

        let content = PDFPageView(
            trade: trade, document: document,
            biz: biz ?? trade.biz,
            bizSub: bizSub ?? trade.bizSub,
            docDate: docDate ?? trade.docDate,
            branding: branding
        )
        .frame(width: pageSize.width, height: pageSize.height)

        let renderer = ImageRenderer(content: content)
        renderer.scale = 3
        guard let image = renderer.uiImage else { return nil }

        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("\(trade.docNo).pdf")
        let pdf = UIGraphicsPDFRenderer(bounds: CGRect(origin: .zero, size: pageSize))
        do {
            try pdf.writePDF(to: url) { ctx in
                ctx.beginPage()
                image.draw(in: CGRect(origin: .zero, size: pageSize))
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

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Letterhead(
                biz: biz,
                bizSub: bizSub,
                docKind: trade.docKind,
                docNo: trade.docNo,
                docDate: docDate,
                branding: branding
            )
            ForEach(document.rows) { row in
                DocRowView(row: row)
            }
            TotalRow(key: document.totalKey, value: document.totalValue, gaps: document.gapCount)
                .padding(.top, 2)
            Spacer(minLength: 0)
            // Free-tier footer ("PREPARED WITH JEFE"); nil once removed (Pro).
            if let footer = branding.footerText {
                Text(footer)
                    .font(Theme.F.mono(7))
                    .tracking(1.6)
                    .foregroundStyle(Theme.C.ink35)
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
