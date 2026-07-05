import SwiftUI

// MARK: - Jobs board row — one line per site, airport-board discipline

struct JobRow: View {
    let job: JobFixture

    var body: some View {
        HStack(spacing: 12) {
            Text(job.time)
                .font(Theme.F.mono(11, .medium))
                .foregroundStyle(Theme.C.ink)
                .frame(width: 46, alignment: .leading)
            VStack(alignment: .leading, spacing: 1) {
                Text(job.name)
                    .font(Theme.F.ui(14.5, .semibold))
                    .strikethrough(job.done)
                    .lineLimit(1)
                Text(job.sub)
                    .font(Theme.F.cond(11.5, .medium))
                    .foregroundStyle(Theme.C.ink60)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            FieldTag(tag: job.tag)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.vertical, 13)
        .opacity(job.done ? 0.45 : 1)
        .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }
    }
}

// MARK: - Live board row — items tick in as they're spoken

struct CapturedRow: View {
    let item: CapturedFixture

    var body: some View {
        HStack(spacing: 10) {
            FieldTag(tag: item.tag)
            Text(item.text)
                .font(Theme.F.cond(13, .semibold))
                .lineLimit(1)
            if item.photos > 0 {
                PhotoChip(count: item.photos)
            }
            Spacer(minLength: 8)
            Text(item.right)
                .font(Theme.F.mono(10.5))
                .foregroundStyle(Theme.C.ink60)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.vertical, 10)
        .overlay(alignment: .bottom) { Theme.C.hairlineSoft.frame(height: 1) }
    }
}

// MARK: - Section head (label left, counter right, heavy rule under)

struct SectionHead: View {
    let left: String
    let right: String
    var rightColor: Color = Theme.C.ink60
    var heavyRule: Bool = true

    var body: some View {
        HStack {
            SectionLabel(left)
            Spacer()
            SectionLabel(right, color: rightColor)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 10)
        .padding(.bottom, 8)
        .overlay(alignment: .bottom) {
            (heavyRule ? Theme.C.ink : Theme.C.hairline)
                .frame(height: heavyRule ? 1.5 : 1)
        }
    }
}
