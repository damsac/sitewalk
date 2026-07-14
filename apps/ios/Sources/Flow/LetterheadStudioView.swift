import SwiftUI
import PhotosUI
import UIKit

// The Letterhead Studio — brand the exported document (design doc §5 / PR #207,
// the STYLE half): logo, brand color, letterhead font, contact lines, and the
// free-tier footer. Edits a working COPY of Branding with a live preview and
// commits on Save. Reached from the board header, same sheet pattern as the
// Vocabulary editor. STRUCTURE (sections / custom fields / uploads) is separate,
// pending dam's core answers.
struct LetterheadStudioView: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var draft: Branding
    @State private var draftProfile: BusinessProfile
    @State private var logoItem: PhotosPickerItem?
    /// A freshly picked logo, held in memory until Save — bytes only hit disk
    /// on commit, so pick-then-cancel leaves no orphan file behind.
    @State private var pickedLogoData: Data?
    @State private var draftLayout: DocumentLayout

    init(model: AppModel) {
        self.model = model
        _draft = State(initialValue: model.branding)
        _draftLayout = State(initialValue: model.documentLayout)
        // The letterhead's business identity (name / city / license) is set at
        // onboarding but belongs here too — this is the one place to edit
        // everything ON the letterhead. Seed from the profile, or a blank one on
        // the no-profile demo path (saving a name creates the profile).
        _draftProfile = State(initialValue: model.profile ?? BusinessProfile(
            businessName: "", cityState: "", licenseNumber: nil, tradeKey: model.trade.key
        ))
    }

    // Curated for v1 (design decision): a handful of brand colors + two bundled
    // faces. "Bring your own font" and more presets are follow-ups.
    private let accents: [UInt32] = [0x9A6A00, 0x3E6B35, 0x2E5A78, 0xA63A2E, 0x141412]
    private let fonts: [(key: String, label: String)] = [("serif", "SOURCE SERIF"), ("sans", "BARLOW")]
    // Trade type labels mirror onboarding (no display name on TradeFixture).
    private let trades: [(key: String, label: String)] = [
        ("landscape", "LANDSCAPE"), ("property", "PROPERTY MGMT"), ("inspection", "INSPECTION"),
    ]

    // The trade the preview renders — follows the draft, so switching it re-keys
    // the doc-kind + sample rows live.
    private var previewTrade: TradeFixture { draftProfile.trade }

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    preview
                    businessSection
                    tradeSection
                    logoSection
                    accentSection
                    fontSection
                    contactSection
                    documentSection
                    watermarkToggle
                }
                .padding(.bottom, 22)
            }
            .background(Theme.C.paperDeep)
            saveBar
        }
        .background(Theme.C.paper.ignoresSafeArea())
        .onChange(of: logoItem) { _, item in
            guard let item else { return }
            Task {
                if let data = try? await item.loadTransferable(type: Data.self) {
                    pickedLogoData = data
                }
                logoItem = nil
            }
        }
    }

    // The logo the preview (and the buttons) should reflect: a fresh in-memory
    // pick wins over the draft's committed filename.
    private var pickedLogoImage: UIImage? { pickedLogoData.flatMap { UIImage(data: $0) } }
    private var hasLogo: Bool { pickedLogoData != nil || draft.logoFilename != nil }
    private var previewLogo: UIImage? { pickedLogoImage ?? draft.logoImage }

    // MARK: Header / Save

    private var header: some View {
        HStack {
            Button { dismiss() } label: {
                Text("CANCEL")
                    .font(Theme.F.mono(9, .semibold)).tracking(1.0)
                    .foregroundStyle(Theme.C.ink60)
            }
            .buttonStyle(.plain)
            Spacer()
            Text("LETTERHEAD")
                .font(Theme.F.mono(9, .semibold)).tracking(2.0)
                .foregroundStyle(Theme.C.orangeDeep)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 16)
        .padding(.bottom, 12)
        .overlay(alignment: .bottom) { Theme.C.ink.frame(height: 2) }
    }

    private var saveBar: some View {
        Button {
            commitAndDismiss()
        } label: {
            Text("SAVE LETTERHEAD")
                .font(Theme.F.ui(15, .bold)).tracking(1.0)
                .foregroundStyle(Theme.C.onOrange)
                .frame(maxWidth: .infinity).frame(height: 54)
                .background(RoundedRectangle(cornerRadius: Theme.S.radius).fill(Theme.C.orange))
        }
        .buttonStyle(.plain)
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 12).padding(.bottom, 12)
        .background(Theme.C.paper)
        .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
    }

    /// The commit path: logo bytes hit disk only here (pick-then-cancel never
    /// writes), and a replaced/removed logo's old file is deleted so orphans
    /// don't accumulate. File I/O runs off the main actor (Branding helpers).
    private func commitAndDismiss() {
        var branding = draft
        let picked = pickedLogoData
        let previous = model.branding.logoFilename
        let profile = draftProfile
        let layout = draftLayout
        Task {
            if let picked, let name = await Branding.saveLogo(picked) {
                branding.logoFilename = name
            }
            if let previous, previous != branding.logoFilename {
                await Branding.deleteLogo(previous)
            }
            model.saveBranding(branding)
            // Only persist the profile when there's a name to carry — avoids
            // minting an empty profile from the no-profile demo path.
            if !profile.businessName.trimmingCharacters(in: .whitespaces).isEmpty {
                model.saveProfile(profile)
            }
            model.saveDocumentLayout(layout)
            dismiss()
        }
    }

    // MARK: Live preview — a real branded document head, re-rendered on edit

    private var preview: some View {
        VStack(alignment: .leading, spacing: 0) {
            Letterhead(
                biz: draftProfile.businessName.isEmpty ? previewTrade.biz : draftProfile.businessName,
                bizSub: draftProfile.letterheadSub,
                docKind: previewTrade.docKind,
                docNo: previewTrade.docNo,
                docDate: model.letterheadDate,
                branding: draft,
                logoOverride: pickedLogoImage
            )
            ForEach(previewTrade.rows.prefix(2)) { DocRowView(row: $0) }
            TotalRow(key: previewTrade.totalKey, value: previewTrade.totalValue, gaps: 0)
                .padding(.top, 2)
            if !draftLayout.termsText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                TermsBlock(text: draftLayout.termsText)
            }
            if draftLayout.showSignature {
                SignatureRow()
            }
            if let footer = draft.footerText {
                Text(footer)
                    .font(Theme.F.mono(7)).tracking(1.6)
                    .foregroundStyle(Theme.C.ink35)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 12)
            }
        }
        .padding(16)
        .background(Theme.C.sheet)
        .compositingGroup()
        .shadow(color: Theme.C.ink.opacity(0.12), radius: 1, y: 1)
        .shadow(color: Theme.C.ink.opacity(0.16), radius: 12, y: 8)
        .padding(.horizontal, 14)
        .padding(.top, 16)
        .padding(.bottom, 4)
    }

    // MARK: Sections

    // Business identity — the letterhead's name line, editable here (not just at
    // onboarding). Names take title/word case; the license is upper-cased.
    private var businessSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            sectionLabel("BUSINESS")
            profileField("NAME", text: $draftProfile.businessName,
                         placeholder: "Summit Lawn & Snow", caps: .words)
            profileField("CITY/ST", text: $draftProfile.cityState,
                         placeholder: "Denver CO", caps: .words)
            profileField("LICENSE", text: Binding(
                get: { draftProfile.licenseNumber ?? "" },
                set: { draftProfile.licenseNumber = $0.isEmpty ? nil : $0 }
            ), placeholder: "44-0781", caps: .characters)
        }
        .padding(.horizontal, Theme.S.screenPad).padding(.top, 18)
    }

    private func profileField(_ key: String, text: Binding<String>, placeholder: String,
                              caps: TextInputAutocapitalization) -> some View {
        HStack(spacing: 10) {
            Text(key)
                .font(Theme.F.mono(8, .semibold)).tracking(1.0)
                .foregroundStyle(Theme.C.ink35)
                .frame(width: 56, alignment: .leading)
            TextField(placeholder, text: text)
                .font(Theme.F.cond(13, .medium))
                .textInputAutocapitalization(caps)
                .autocorrectionDisabled()
        }
        .padding(.horizontal, 11).padding(.vertical, 10)
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(Theme.C.hairline, lineWidth: 1.5))
    }

    // Trade — changes the document types (estimate vs inspection) and the board,
    // not just letterhead text; the preview's doc-kind follows it. Saved via the
    // profile (reloadProfile re-keys model.trade + jobs).
    private var tradeSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionLabel("TRADE")
            Menu {
                ForEach(trades, id: \.key) { trade in
                    Button(trade.label) { draftProfile.tradeKey = trade.key }
                }
            } label: {
                HStack {
                    Text(trades.first { $0.key == draftProfile.tradeKey }?.label
                         ?? draftProfile.tradeKey.uppercased())
                        .font(Theme.F.cond(13, .semibold))
                        .foregroundStyle(Theme.C.ink)
                    Spacer()
                    Text("⌄").font(Theme.F.mono(12)).foregroundStyle(Theme.C.ink60)
                }
                .padding(.horizontal, 11).padding(.vertical, 11)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(Theme.C.hairline, lineWidth: 1.5))
            }
        }
        .padding(.horizontal, Theme.S.screenPad).padding(.top, 18)
    }

    private var logoSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionLabel("LOGO")
            HStack(spacing: 11) {
                ZStack {
                    RoundedRectangle(cornerRadius: 4).stroke(Theme.C.hairline, lineWidth: 1.5)
                    if let logo = previewLogo {
                        Image(uiImage: logo).resizable().scaledToFit().padding(6)
                    } else {
                        Text("NONE").font(Theme.F.mono(8)).foregroundStyle(Theme.C.ink35)
                    }
                }
                .frame(width: 54, height: 54)
                PhotosPicker(selection: $logoItem, matching: .images) {
                    Text(hasLogo ? "REPLACE" : "ADD LOGO")
                        .font(Theme.F.mono(9, .semibold)).tracking(1.0)
                        .foregroundStyle(Theme.C.paper)
                        .padding(.horizontal, 14).frame(height: 40)
                        .background(RoundedRectangle(cornerRadius: 8).fill(Theme.C.ink))
                }
                .buttonStyle(.plain)
                if hasLogo {
                    Button {
                        draft.logoFilename = nil
                        pickedLogoData = nil
                    } label: {
                        Text("REMOVE")
                            .font(Theme.F.mono(9, .semibold)).tracking(1.0)
                            .foregroundStyle(Theme.C.ink60)
                            .padding(.horizontal, 12).frame(height: 40)
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.C.hairline, lineWidth: 1.5))
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
            }
        }
        .padding(.horizontal, Theme.S.screenPad).padding(.top, 16)
    }

    private var accentSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionLabel("BRAND COLOR")
            HStack(spacing: 12) {
                ForEach(accents, id: \.self) { hex in
                    Button { draft.accentHex = hex } label: {
                        Circle().fill(Color(hex: hex))
                            .frame(width: 34, height: 34)
                            .overlay(Circle().stroke(Theme.C.ink, lineWidth: draft.accentHex == hex ? 2.5 : 0))
                            .padding(2)
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
            }
        }
        .padding(.horizontal, Theme.S.screenPad).padding(.top, 18)
    }

    private var fontSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionLabel("LETTERHEAD FONT")
            HStack(spacing: 8) {
                ForEach(fonts, id: \.key) { font in
                    Button { draft.fontKey = font.key } label: {
                        Text(font.label)
                            .font(Theme.F.mono(9, .semibold)).tracking(0.6)
                            .foregroundStyle(draft.fontKey == font.key ? Theme.C.ink : Theme.C.ink60)
                            .padding(.horizontal, 12).frame(height: 38)
                            .overlay(RoundedRectangle(cornerRadius: 6)
                                .stroke(draft.fontKey == font.key ? Theme.C.ink : Theme.C.hairline, lineWidth: 1.5))
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
            }
        }
        .padding(.horizontal, Theme.S.screenPad).padding(.top, 18)
    }

    private var contactSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            sectionLabel("CONTACT")
            contactField("PHONE", text: $draft.phone, keyboard: .phonePad)
            contactField("EMAIL", text: $draft.email, keyboard: .emailAddress)
            contactField("WEB", text: $draft.website, keyboard: .URL)
        }
        .padding(.horizontal, Theme.S.screenPad).padding(.top, 18)
    }

    private func contactField(_ key: String, text: Binding<String>, keyboard: UIKeyboardType) -> some View {
        HStack(spacing: 10) {
            Text(key)
                .font(Theme.F.mono(8, .semibold)).tracking(1.0)
                .foregroundStyle(Theme.C.ink35)
                .frame(width: 46, alignment: .leading)
            TextField("", text: text)
                .font(Theme.F.cond(13, .medium))
                .keyboardType(keyboard)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
        }
        .padding(.horizontal, 11).padding(.vertical, 10)
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(Theme.C.hairline, lineWidth: 1.5))
    }

    // Document structure basics (app-side): operator terms + a signature line.
    // Richer, LLM-filled structure is dam's core DocumentSchema seam (§7.2, v2).
    private var documentSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionLabel("DOCUMENT")
            VStack(alignment: .leading, spacing: 5) {
                Text("TERMS / PAYMENT")
                    .font(Theme.F.mono(7.5, .semibold)).tracking(1.0)
                    .foregroundStyle(Theme.C.ink35)
                TextField("50% deposit to schedule · balance on completion · quote valid 30 days",
                          text: $draftLayout.termsText, axis: .vertical)
                    .font(Theme.F.cond(13, .medium))
                    .lineLimit(3...6)
                    .textInputAutocapitalization(.sentences)
                    .padding(.horizontal, 11).padding(.vertical, 10)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(Theme.C.hairline, lineWidth: 1.5))
            }
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Client signature line")
                        .font(Theme.F.cond(13, .semibold))
                    Text("A SIGN + DATE LINE AT THE FOOT OF THE DOCUMENT")
                        .font(Theme.F.mono(7.5)).tracking(0.4)
                        .foregroundStyle(Theme.C.ink60)
                }
                Spacer()
                Toggle("", isOn: $draftLayout.showSignature).labelsHidden().tint(Theme.C.orange)
            }
            .padding(.horizontal, 12).padding(.vertical, 11)
            .background(Theme.C.sheet)
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.C.hairline, lineWidth: 1.5))
        }
        .padding(.horizontal, Theme.S.screenPad).padding(.top, 18)
    }

    private var watermarkToggle: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text("“Prepared with Jefe” footer")
                    .font(Theme.F.cond(13, .semibold))
                Text("REMOVING IT IS A JEFE PRO FEATURE")
                    .font(Theme.F.mono(7.5)).tracking(0.4)
                    .foregroundStyle(Theme.C.ink60)
            }
            Spacer()
            Toggle("", isOn: $draft.showWatermark).labelsHidden().tint(Theme.C.orange)
        }
        .padding(.horizontal, 12).padding(.vertical, 12)
        .background(Theme.C.sheet)
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.C.hairline, lineWidth: 1.5))
        .padding(.horizontal, Theme.S.screenPad).padding(.top, 18)
    }

    private func sectionLabel(_ text: String) -> some View {
        Text(text)
            .font(Theme.F.mono(8.5, .semibold)).tracking(2.0)
            .foregroundStyle(Theme.C.ink60)
    }
}
