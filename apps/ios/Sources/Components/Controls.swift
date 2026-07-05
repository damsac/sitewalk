import SwiftUI

// MARK: - Pressed-block button style (mechanical, no bounce)

private struct BlockButton: View {
    let title: String
    let fill: Color
    let textColor: Color
    let shadow: Color
    var leadingDot: Bool = false
    var height: CGFloat = Theme.S.buttonHeight

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: Theme.S.radius)
                .fill(shadow)
                .offset(y: 3)
            RoundedRectangle(cornerRadius: Theme.S.radius)
                .fill(fill)
            HStack(spacing: 12) {
                if leadingDot {
                    Circle().fill(textColor).frame(width: 10, height: 10)
                }
                Text(title)
                    .font(Theme.F.ui(15, .bold))
                    .tracking(1.4)
            }
            .foregroundStyle(textColor)
        }
        .frame(height: height)
    }
}

// MARK: - START WALK — the biggest thing on the home screen

struct WalkButton: View {
    var title: String = "START WALK"

    var body: some View {
        BlockButton(
            title: title,
            fill: Theme.C.orange,
            textColor: Theme.C.onOrange,
            shadow: Theme.C.orangeDeep,
            leadingDot: true
        )
    }
}

// MARK: - Capture controls: PHOTO square + PAUSE outline + DONE block

struct PhotoSquareButton: View {
    var body: some View {
        VStack(spacing: 3) {
            Image(systemName: "camera")
                .font(.system(size: 17, weight: .medium))
            Text("PHOTO")
                .font(Theme.F.mono(7, .semibold))
                .tracking(1.1)
        }
        .foregroundStyle(Theme.C.ink)
        .frame(width: Theme.S.buttonHeight, height: Theme.S.buttonHeight)
        .overlay(
            RoundedRectangle(cornerRadius: Theme.S.radius)
                .stroke(Theme.C.ink, lineWidth: 2)
        )
    }
}

struct PauseButton: View {
    var body: some View {
        Text("PAUSE")
            .font(Theme.F.ui(13.5, .bold))
            .tracking(1.1)
            .foregroundStyle(Theme.C.ink)
            .frame(maxWidth: .infinity)
            .frame(height: Theme.S.buttonHeight)
            .overlay(
                RoundedRectangle(cornerRadius: Theme.S.radius)
                    .stroke(Theme.C.ink, lineWidth: 2)
            )
    }
}

struct DoneButton: View {
    var body: some View {
        BlockButton(
            title: "DONE",
            fill: Theme.C.ink,
            textColor: Theme.C.paper,
            shadow: .black
        )
    }
}

struct CaptureControls: View {
    var body: some View {
        HStack(spacing: 9) {
            PhotoSquareButton()
            PauseButton()
                .frame(width: 96)
            DoneButton()
                .frame(maxWidth: .infinity)
        }
    }
}

// MARK: - Review bar: ADJUST outline + SEND block

struct ReviewBar: View {
    let sendTitle: String

    var body: some View {
        HStack(spacing: 10) {
            Text("ADJUST")
                .font(Theme.F.ui(14, .bold))
                .tracking(1.1)
                .foregroundStyle(Theme.C.ink)
                .frame(width: 124)
                .frame(height: 58)
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.S.radius)
                        .stroke(Theme.C.ink, lineWidth: 2)
                )
            BlockButton(
                title: sendTitle,
                fill: Theme.C.orange,
                textColor: Theme.C.onOrange,
                shadow: Theme.C.orangeDeep,
                height: 58
            )
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
