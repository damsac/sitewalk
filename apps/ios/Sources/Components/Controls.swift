import SwiftUI

// MARK: - The control system: raised, recessed, or off
//
// ONE RULE, from the control-system review (2026-07-29): if it can be pressed,
// it stands off the paper or is cut into it. Flat is reserved for DISABLED.
// Then flatness carries meaning instead of just reading as print.
//
// Why this is a ButtonStyle and not a View: the old `BlockButton` hand-stacked
// two rounded rectangles inside a `View`, which structurally cannot see
// `configuration.isPressed`. Every button in the app also carried
// `.buttonStyle(.plain)` — added to stop iOS tinting labels blue, which also
// threw away all press feedback. Between them, NOTHING in the app responded to
// touch, and that absence is most of the "feels flat" complaint. Moving the
// same drawing into a ButtonStyle is what unlocks it.

/// Tier 1 + 2: a cap sitting on a darker lip. On touch-down the cap travels the
/// 3pt onto its lip and the lip vanishes — 90ms ease-out, no scale, no bounce,
/// which is exactly how a machine button behaves.
///
/// Disabled removes the lip entirely: the control becomes physically flat, and
/// flat is the one state that is never pressable.
struct RaisedBlockStyle: ButtonStyle {
    var face: Color = Theme.C.orange
    var lip: Color = Theme.C.orangeDeep
    var text: Color = Theme.C.onOrange
    /// Tier 2 / destructive outline. `nil` = filled, no border.
    var border: Color?
    var height: CGFloat = Theme.S.buttonHeight
    var radius: CGFloat = Theme.S.radius
    /// The live-state dot on START WALK / SEND.
    var leadingDot: Bool = false
    /// Full-width by default (bottom-anchored primaries). Header chips size to
    /// their label instead — forcing `.infinity` on those made the label
    /// overflow its own body.
    var fillWidth: Bool = true

    @Environment(\.isEnabled) private var isEnabled

    private static let lipDepth: CGFloat = 3

    func makeBody(configuration: Configuration) -> some View {
        let down = configuration.isPressed
        let lipHeight: CGFloat = isEnabled ? (down ? 0 : Self.lipDepth) : 0
        return HStack(spacing: 12) {
            if leadingDot, isEnabled {
                Circle().fill(text).frame(width: 10, height: 10)
            }
            configuration.label
        }
        .foregroundStyle(isEnabled ? text : Theme.C.ink35)
        .padding(.horizontal, fillWidth ? 0 : 14)
        .frame(maxWidth: fillWidth ? .infinity : nil)
        .frame(height: height - Self.lipDepth)
        .background(isEnabled ? face : Theme.C.paperDeep)
        .overlay(
            RoundedRectangle(cornerRadius: radius)
                .stroke(
                    isEnabled ? (border ?? .clear) : Theme.C.hairline,
                    lineWidth: border == nil ? 1 : 2
                )
        )
        .clipShape(RoundedRectangle(cornerRadius: radius))
        // The travel: padding moves the cap down onto the lip rather than
        // scaling it, so the label never distorts.
        .padding(.top, down ? Self.lipDepth : 0)
        .padding(.bottom, lipHeight)
        .background(isEnabled ? lip : Color.clear)
        .clipShape(RoundedRectangle(cornerRadius: radius))
        .frame(height: height)
        .animation(.easeOut(duration: 0.09), value: down)
        // Gloves get almost no tactile confirmation off glass. Fires on
        // touch-DOWN only — a second tick on release reads as a double-tap.
        .sensoryFeedback(trigger: down) { _, isDown in
            isDown ? .impact(flexibility: .rigid) : nil
        }
    }
}

/// Tier 3: the inverse gesture — a control cut INTO the sheet, for things that
/// live inside a row (FILE, SHOW ALL, CLOSE, the plan chip). A recessed well
/// with a hairline shadow across its top edge, like a sunk key on a tool case.
///
/// Replaces every dashed box and every bare-text tap target. Dashed reads
/// "empty placeholder" in app language, which is the opposite of "tap me" —
/// and Isaac reported that three separate times.
struct WellChipStyle: ButtonStyle {
    var tint: Color = Theme.C.paperDeep
    var text: Color = Theme.C.ink
    var minHeight: CGFloat = 44

    func makeBody(configuration: Configuration) -> some View {
        let down = configuration.isPressed
        return configuration.label
            .foregroundStyle(text)
            .frame(minHeight: minHeight)
            .padding(.horizontal, 12)
            .background(down ? tint.opacity(0.72) : tint)
            // The "cut in" cue: a hairline across the TOP edge only, reading as
            // the shadow the sheet casts into the well.
            .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1.5) }
            .overlay(RoundedRectangle(cornerRadius: Theme.S.radiusCard).stroke(Theme.C.hairline))
            .clipShape(RoundedRectangle(cornerRadius: Theme.S.radiusCard))
            // Invisible hit slop — the chip reads small but catches a glove.
            .contentShape(Rectangle().inset(by: -8))
            .animation(.easeOut(duration: 0.09), value: down)
            .sensoryFeedback(trigger: down) { _, isDown in
                isDown ? .impact(flexibility: .rigid) : nil
            }
    }
}

/// Tier 4: a list row. The humblest control in the app, and until now not a
/// button at all — a hairline underline with no fill under the finger and no
/// chevron. `Theme.S.minTarget` was already declared 56 and simply not applied.
struct FieldRowStyle: ButtonStyle {
    var minHeight: CGFloat = Theme.S.minTarget

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(minHeight: minHeight)
            .background(configuration.isPressed ? Theme.C.paperDeep : Color.clear)
            .contentShape(Rectangle())
            .animation(.easeOut(duration: 0.09), value: configuration.isPressed)
    }
}

/// Tier 5: a bare tap that still answers. Looks exactly like `.plain` — no
/// chrome, no fill, no travel — but dims under the finger and ticks on
/// touch-down like every other control.
///
/// It exists because of what the tick MEANS. Isaac, 2026-08-12: *"I like how
/// START WALK and my business have a physical click feel… can we make the
/// other buttons like that? Maybe not every single one but the ones that make
/// sense."* The ones that make sense are the ones where something HAPPENS —
/// a line saved or removed, notes exported, a photo deleted. A tick is the
/// phone saying "that landed", which is worth most when the screen's answer
/// is a sheet closing behind your own thumb, or when you are wearing gloves
/// and looking at a yard instead of the glass.
///
/// Deliberately NOT applied to CLOSE, CANCEL, `‹ NOTES`, or a photo zoom.
/// Those navigate: nothing committed, nothing to confirm, and a tick that
/// fires when nothing happened teaches the hand to stop trusting it. Same
/// reason `FieldRowStyle` (tier 4) stays silent — twenty job rows buzzing
/// past under a scrolling thumb is noise, and worse, it fires on the drag
/// that was meant to scroll.
struct BareTapStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        let down = configuration.isPressed
        return configuration.label
            .opacity(down ? 0.55 : 1)
            .contentShape(Rectangle())
            .animation(.easeOut(duration: 0.09), value: down)
            // Touch-down only, matching the other tiers — a second tick on
            // release reads as a double-tap.
            .sensoryFeedback(trigger: down) { _, isDown in
                isDown ? .impact(flexibility: .rigid) : nil
            }
    }
}

// MARK: - Tier presets
//
// Named so a call site picks a TIER, never a colour. One amber fill per screen
// is a rule the styles can enforce only if nobody hand-rolls a face colour.

extension ButtonStyle where Self == RaisedBlockStyle {
    /// Tier 1 — the one primary action on a screen. START WALK / SEND / SAVE.
    static var primaryBlock: RaisedBlockStyle {
        RaisedBlockStyle(leadingDot: true)
    }

    /// Tier 1, ink — DONE. Same physics, no amber, because the walk screen's
    /// amber is already spent on the RECORDING banner.
    static var inkBlock: RaisedBlockStyle {
        RaisedBlockStyle(face: Theme.C.ink, lip: .black, text: Theme.C.paper, border: nil)
    }

    /// Tier 2 — secondary. Same body, no colour: PAUSE, PHOTO, ADJUST,
    /// My business, Add job.
    static var secondaryBlock: RaisedBlockStyle {
        RaisedBlockStyle(
            face: Theme.C.sheet, lip: Theme.C.ink.opacity(0.32),
            text: Theme.C.ink, border: Theme.C.ink
        )
    }

    /// Tier 2 at header-chip height.
    static var secondaryChip: RaisedBlockStyle {
        RaisedBlockStyle(
            face: Theme.C.sheet, lip: Theme.C.ink.opacity(0.32),
            text: Theme.C.ink, border: Theme.C.ink, height: 44, radius: 4,
            fillWidth: false
        )
    }

    /// Tier 4 — destructive. Tier 2's body in red, because flat red text
    /// didn't read as tappable (Isaac, on DISCARD).
    static var destructiveBlock: RaisedBlockStyle {
        RaisedBlockStyle(
            face: Theme.C.sheet, lip: Theme.C.redTag.opacity(0.32),
            text: Theme.C.redTag, border: Theme.C.redTag, height: 52, radius: 4
        )
    }
}

/// The Tier 3 well drawn as chrome rather than as a style.
///
/// `Menu` does not route its label through a `ButtonStyle`, so a menu that
/// should look like a chip (FILE, the trade picker) needs the same recessed
/// treatment applied directly. Press feedback comes from the menu's own
/// highlight; everything else matches `WellChipStyle` exactly, and both must be
/// changed together.
struct WellChrome: ViewModifier {
    var tint: Color = Theme.C.paperDeep
    var minHeight: CGFloat = 44

    func body(content: Content) -> some View {
        content
            // EXACT height, not a minimum. `minHeight` let an inline chip
            // stretch to fill whatever row it sat in — on the walk row's
            // metadata line that rendered a ~130pt tall FILE button.
            .frame(height: minHeight)
            .padding(.horizontal, 12)
            .background(tint)
            .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1.5) }
            .overlay(RoundedRectangle(cornerRadius: Theme.S.radiusCard).stroke(Theme.C.hairline))
            .clipShape(RoundedRectangle(cornerRadius: Theme.S.radiusCard))
            .contentShape(Rectangle().inset(by: -8))
    }
}

extension View {
    /// Tier 3 chrome for controls that can't take a `ButtonStyle` (menus).
    func wellChrome(tint: Color = Theme.C.paperDeep, minHeight: CGFloat = 44) -> some View {
        modifier(WellChrome(tint: tint, minHeight: minHeight))
    }
}

extension ButtonStyle where Self == WellChipStyle {
    /// Tier 3 — recessed in-row chip.
    static var wellChip: WellChipStyle { WellChipStyle() }
}

extension ButtonStyle where Self == FieldRowStyle {
    /// Tier 4 — a list row with a fill under the finger.
    static var fieldRow: FieldRowStyle { FieldRowStyle() }
}

extension ButtonStyle where Self == BareTapStyle {
    /// Tier 5 — an unchromed tap that COMMITS something. Use `.plain` when it
    /// only navigates.
    static var bareTap: BareTapStyle { BareTapStyle() }
}

// MARK: - Labels
//
// These are LABEL CONTENT ONLY — the body, lip and press behaviour all come
// from the styles above. `BlockButton` (a View that hand-stacked two rounded
// rectangles) is gone: it could not see press state, which is the structural
// reason nothing in the app responded to touch.

/// The text inside any raised block.
struct BlockLabel: View {
    let title: String
    var size: CGFloat = 15

    init(_ title: String, size: CGFloat = 15) {
        self.title = title
        self.size = size
    }

    var body: some View {
        Text(title)
            .font(Theme.F.ui(size, .bold))
            .tracking(1.4)
    }
}

/// PHOTO — icon over label, sized square by its call site.
struct PhotoLabel: View {
    var body: some View {
        VStack(spacing: 3) {
            Image(systemName: "camera")
                .font(.system(size: 19, weight: .medium))
            Text("PHOTO")
                .font(Theme.F.mono(9, .semibold))
                .tracking(1.1)
        }
    }
}

// MARK: - Capture controls: PHOTO + PAUSE + DONE
//
// Static composition for the gallery/screens. The live walk screen builds its
// own so each control gets a real action.

struct CaptureControls: View {
    var body: some View {
        HStack(spacing: 9) {
            Button {} label: { PhotoLabel() }
                .buttonStyle(.secondaryBlock)
                .frame(width: Theme.S.buttonHeight)
            Button {} label: { BlockLabel("PAUSE", size: 13.5) }
                .buttonStyle(.secondaryBlock)
                .frame(width: 96)
            Button {} label: { BlockLabel("DONE") }
                .buttonStyle(.inkBlock)
                .frame(maxWidth: .infinity)
        }
    }
}

// MARK: - Review bar: ADJUST + SEND

struct ReviewBar: View {
    let sendTitle: String

    var body: some View {
        HStack(spacing: 10) {
            Button {} label: { BlockLabel("ADJUST", size: 14) }
                .buttonStyle(RaisedBlockStyle(
                    face: Theme.C.sheet, lip: Theme.C.ink.opacity(0.32),
                    text: Theme.C.ink, border: Theme.C.ink, height: 58
                ))
                .frame(width: 124)
            Button {} label: { BlockLabel(sendTitle) }
                .buttonStyle(RaisedBlockStyle(height: 58, leadingDot: false))
                .frame(maxWidth: .infinity)
        }
    }
}

// MARK: - Recording banner — readable at arm's length

struct RecBanner: View {
    let timer: String
    @State private var on = true

    var body: some View {
        HStack {
            HStack(spacing: 9) {
                Circle()
                    .fill(Theme.C.onOrange)
                    .frame(width: 9, height: 9)
                    .opacity(on ? 1 : 0.25)
                    .animation(.easeInOut(duration: 0.55).repeatForever(), value: on)
                    .onAppear { on = false }
                Text("RECORDING")
                    .font(Theme.F.ui(12, .bold))
                    .tracking(2.2)
            }
            Spacer()
            Text(timer)
                .font(Theme.F.mono(13, .semibold))
        }
        .foregroundStyle(Theme.C.onOrange)
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.vertical, 9)
        .background(Theme.C.orange, ignoresSafeAreaEdges: [])
    }
}

// MARK: - Waveform (live capture)

struct Waveform: View {
    var barCount: Int = 44

    var body: some View {
        TimelineView(.animation(minimumInterval: 0.1)) { ctx in
            let t = ctx.date.timeIntervalSinceReferenceDate
            HStack(spacing: 2.5) {
                ForEach(0..<barCount, id: \.self) { i in
                    let phase = t * (1.4 + Double(i % 7) * 0.33) + Double(i) * 0.7
                    let h = 4 + 22 * abs(sin(phase))
                    Capsule()
                        .fill(Theme.C.orange)
                        .frame(width: 3, height: h)
                }
            }
            .frame(maxWidth: .infinity)
        }
        .frame(height: 30)
    }
}
