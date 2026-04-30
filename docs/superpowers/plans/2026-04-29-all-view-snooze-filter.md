# All View Snooze Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a session-only snooze visibility toggle to the All view of the home Focus tab, with snoozed entries mixed inline in their normal category sections and visually marked.

**Architecture:** Single `@State` toggle in `AllEntriesView`. `RootView` exposes a separate `snoozedEntries` array; `AllEntriesView` merges with `activeEntries` when toggle is on. `SmartListRow` renders a snoozed treatment (icon swap, "Until X" subtitle, opacity 0.7). Swipe action transforms from "Snooze" to "Wake now" for snoozed rows. New `EntryAction.wake` case mirrors `unarchive`. Analytics event fires on every toggle press.

**Tech Stack:** SwiftUI, SwiftData, StudioAnalytics SDK.

**Spec:** `docs/superpowers/specs/2026-04-29-all-view-snooze-filter-design.md`.

---

## File Map

- **Modify:** `Murmur/Models/Entry.swift` — add `EntryAction.wake` case + handling
- **Modify:** `Murmur/Views/RootView.swift` — add `snoozedEntries` computed prop, pass to `AllEntriesView`, route `.wake`
- **Modify:** `Murmur/Views/Home/AllEntriesView.swift` — toggle state, button beside search, merged source, snoozed mark in `SmartListRow`
- **Modify:** `Murmur/Views/Home/DamHomeView.swift` — wire `snoozedEntries` parameter through to `AllEntriesView`, swap swipe action for snoozed entries
- **Modify:** `Murmur/Views/Home/ZonedFocusHomeView.swift` — same as above
- **Modify:** `Murmur/Services/MurmurEvents.swift` — add `AllViewSnoozeFilterToggled`

---

### Task 1: Add `EntryAction.wake` case

**Files:**
- Modify: `Murmur/Models/Entry.swift:413-475`

- [ ] **Step 1: Add `.wake` case to `EntryAction`**

In `Murmur/Models/Entry.swift` around line 413, add a new case to `EntryAction`:

```swift
enum EntryAction {
    case snooze(until: Date?)  // nil = default 1 hour
    case wake                  // wake a snoozed entry: set status = .active, clear snoozeUntil
    case complete
    case archive
    case unarchive
    case delete
    case checkOffHabit         // toggle done-for-period on habit entries
    case toggleListItem(index: Int) // toggle checkbox on a list item
}
```

- [ ] **Step 2: Implement `.wake` handling in `perform(_:in:preferences:)`**

In `Murmur/Models/Entry.swift`, in the `perform` switch (around line 425), add a `.wake` case after `.snooze`:

```swift
case .wake:
    status = .active
    snoozeUntil = nil
    updatedAt = Date()
    save(in: context)
    NotificationService.shared.cancel(self)
```

`NotificationService.shared.cancel(self)` removes any pending snooze-wake-up notification (which is now obsolete since we're waking it manually).

- [ ] **Step 3: Build to verify it compiles**

Run: `make build`
Expected: build succeeds.

- [ ] **Step 4: Commit**

```bash
git add Murmur/Models/Entry.swift
git commit -m "feat(entries): add EntryAction.wake for manual snooze wake"
```

---

### Task 2: Add `AllViewSnoozeFilterToggled` analytics event

**Files:**
- Modify: `Murmur/Services/MurmurEvents.swift`

- [ ] **Step 1: Add event struct**

Append to `Murmur/Services/MurmurEvents.swift`:

```swift
// MARK: - All View Filters

struct AllViewSnoozeFilterToggled: AnalyticsEvent {
    static let eventName = "all_view.snooze_filter_toggled"
    let enabled: Bool
    let snoozedCount: Int
}
```

- [ ] **Step 2: Build to verify**

Run: `make build`
Expected: build succeeds.

- [ ] **Step 3: Commit**

```bash
git add Murmur/Services/MurmurEvents.swift
git commit -m "feat(analytics): add AllViewSnoozeFilterToggled event"
```

---

### Task 3: Add `snoozedEntries` computed property in `RootView` + route `.wake` action

**Files:**
- Modify: `Murmur/Views/RootView.swift`

- [ ] **Step 1: Add `snoozedEntries` computed property**

In `Murmur/Views/RootView.swift`, after `activeEntries` (around line 645), add:

```swift
var snoozedEntries: [Entry] {
    let pendingReveal = appState.conversation.pendingRevealEntryIDs
    return entries.filter {
        $0.status == .snoozed
            && $0.persistentModelID != pendingDeleteEntry?.persistentModelID
            && !pendingReveal.contains($0.id)
    }
}
```

- [ ] **Step 2: Verify `handleEntryAction` already routes through `entry.perform(...)` and works with `.wake`**

Find the `handleEntryAction` function in `Murmur/Views/RootView.swift` (search for `func handleEntryAction`). It should call `entry.perform(action, in: modelContext, preferences: notifPrefs)` for most actions. Since `.wake` is now in `perform`, no change needed — but verify by reading the function.

If `handleEntryAction` has a switch that needs explicit cases, add a `case .wake:` branch that calls `entry.perform(.wake, in: modelContext, preferences: notifPrefs)` and shows a toast like `"Awake"` or similar.

- [ ] **Step 3: Build to verify**

Run: `make build`
Expected: build succeeds (no usage yet — just verifying syntax).

- [ ] **Step 4: Commit**

```bash
git add Murmur/Views/RootView.swift
git commit -m "feat(root): expose snoozedEntries and route wake action"
```

---

### Task 4: Wire `snoozedEntries` parameter through home view containers

**Files:**
- Modify: `Murmur/Views/Home/DamHomeView.swift`
- Modify: `Murmur/Views/Home/ZonedFocusHomeView.swift`
- Modify: `Murmur/Views/RootView.swift`

- [ ] **Step 1: Add `snoozedEntries` parameter to `DamHomeView`**

In `Murmur/Views/Home/DamHomeView.swift`, find the top-level `DamHomeView` struct and add a `snoozedEntries: [Entry]` property next to `entries: [Entry]`. Wire it through to wherever `AllEntriesView` is constructed inside this file. Pass `snoozedEntries: snoozedEntries` to `AllEntriesView`.

(See `swipeActionsProvider` plumbing as the closest pattern — duplicate the same pass-through.)

- [ ] **Step 2: Same for `ZonedFocusHomeView`**

In `Murmur/Views/Home/ZonedFocusHomeView.swift`, add `snoozedEntries: [Entry]` to the top-level struct and any internal views that reach `AllEntriesView`. Pass through.

- [ ] **Step 3: Pass `snoozedEntries` from `RootView` to both home views**

In `Murmur/Views/RootView.swift` at `homeContent` (around line 388-413), pass `snoozedEntries: snoozedEntries` to both `DamHomeView` and `ZonedFocusHomeView` constructors.

- [ ] **Step 4: Build to verify**

Run: `make build`
Expected: build succeeds — but `AllEntriesView` will error on the new parameter until Task 5. If the error is "missing argument for parameter 'snoozedEntries' in call", proceed to Task 5 first and come back. To avoid this, add a temporary default value: change `AllEntriesView` calls to pass `snoozedEntries: snoozedEntries` after Task 5 lands the parameter — combine these tasks into one commit.

**Combined approach (recommended):** do Tasks 4 and 5 together, commit once.

- [ ] **Step 5: (Combined commit with Task 5) — see Task 5 commit**

---

### Task 5: Add toggle state, merged source, and toggle button to `AllEntriesView`

**Files:**
- Modify: `Murmur/Views/Home/AllEntriesView.swift`

- [ ] **Step 1: Add `snoozedEntries` parameter and `showSnoozed` state**

At the top of `AllEntriesView`, add:

```swift
let snoozedEntries: [Entry]
@State private var showSnoozed = false
```

- [ ] **Step 2: Add merged source computed prop**

Replace the existing `filteredEntries` computed property:

```swift
private var combinedEntries: [Entry] {
    showSnoozed ? entries + snoozedEntries : entries
}

private var filteredEntries: [Entry] {
    let source = combinedEntries
    guard !searchText.isEmpty else { return source }
    let q = searchText.lowercased()
    return source.filter { $0.summary.lowercased().contains(q) }
}
```

- [ ] **Step 3: Replace search bar wrapper with HStack containing search + toggle button**

Replace the `searchBar` body and its single `.padding(...)` call site with this layout. Locate around line 49-53:

```swift
HStack(spacing: 8) {
    searchBar
    snoozeToggleButton
}
.padding(.horizontal, Theme.Spacing.screenPadding)
.padding(.top, 12)
.padding(.bottom, 4)
```

Then add the `snoozeToggleButton` computed view below `searchBar`:

```swift
@ViewBuilder
private var snoozeToggleButton: some View {
    Button {
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        let newValue = !showSnoozed
        StudioAnalytics.track(AllViewSnoozeFilterToggled(
            enabled: newValue,
            snoozedCount: snoozedEntries.count
        ))
        withAnimation(.easeInOut(duration: 0.2)) {
            showSnoozed = newValue
        }
    } label: {
        Image(systemName: showSnoozed ? "moon.zzz.fill" : "moon.zzz")
            .font(.system(size: 16, weight: .medium))
            .foregroundStyle(showSnoozed ? Theme.Colors.accentYellow : Theme.Colors.textTertiary)
            .frame(width: 36, height: 36)
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(Theme.Colors.bgCard)
                    .overlay(
                        RoundedRectangle(cornerRadius: 10)
                            .stroke(Theme.Colors.borderSubtle, lineWidth: 1)
                    )
            )
            .shadow(
                color: showSnoozed ? Theme.Colors.accentYellow.opacity(0.4) : .clear,
                radius: showSnoozed ? 6 : 0
            )
    }
    .buttonStyle(.plain)
    .accessibilityLabel(showSnoozed ? "Hide snoozed entries" : "Show snoozed entries")
}
```

Also add the import at the top of the file if not already present:

```swift
import StudioAnalytics
```

- [ ] **Step 4: Build to verify**

Run: `make build`
Expected: build succeeds. This step depends on Task 4 (parameter wiring through home views) being done in the same edit session.

- [ ] **Step 5: Combined commit (Tasks 4 + 5)**

```bash
git add Murmur/Views/Home/AllEntriesView.swift Murmur/Views/Home/DamHomeView.swift Murmur/Views/Home/ZonedFocusHomeView.swift Murmur/Views/RootView.swift
git commit -m "feat(all-view): snooze filter toggle button and merged source"
```

---

### Task 6: Render snoozed treatment in `SmartListRow`

**Files:**
- Modify: `Murmur/Views/Home/AllEntriesView.swift` (the `SmartListRow` struct, around line 525-629)

- [ ] **Step 1: Add `isSnoozed` and `snoozeText` computed props to `SmartListRow`**

Inside `SmartListRow`, after `dueText` (around line 545), add:

```swift
private var isSnoozed: Bool { entry.status == .snoozed }

private var snoozeText: String? {
    guard isSnoozed, let snoozeUntil = entry.snoozeUntil else { return nil }
    let calendar = Calendar.current
    let formatter = DateFormatter()
    if calendar.isDateInToday(snoozeUntil) {
        formatter.dateFormat = "h:mm a"
        return "Until \(formatter.string(from: snoozeUntil))"
    }
    if calendar.isDateInTomorrow(snoozeUntil) { return "Until tomorrow" }
    formatter.dateFormat = "MMM d"
    return "Until \(formatter.string(from: snoozeUntil))"
}
```

- [ ] **Step 2: Render moon icon in place of the category dot when snoozed**

In the `body` of `SmartListRow`, find the `else` branch that renders the category dot (around line 571-578):

```swift
} else {
    let dotColor = Theme.categoryColor(entry.category)
    Circle()
        .fill(dotColor)
        .frame(width: 8, height: 8)
        .shadow(color: dotColor.opacity(0.6), radius: 4)
        .padding(.leading, 2)
}
```

Replace with:

```swift
} else if isSnoozed {
    let dotColor = Theme.categoryColor(entry.category)
    Image(systemName: "moon.zzz.fill")
        .font(.system(size: 12, weight: .medium))
        .foregroundStyle(dotColor)
        .frame(width: 14, height: 14)
        .shadow(color: dotColor.opacity(0.6), radius: 4)
} else {
    let dotColor = Theme.categoryColor(entry.category)
    Circle()
        .fill(dotColor)
        .frame(width: 8, height: 8)
        .shadow(color: dotColor.opacity(0.6), radius: 4)
        .padding(.leading, 2)
}
```

- [ ] **Step 3: Show `snoozeText` in subtitle line**

In `SmartListRow.body`, find the subtitle block that shows `cadence` or `dueText` (around line 586-594):

```swift
if entry.category == .habit, let cadence = entry.cadence {
    Text(cadence.displayName)
        .font(.caption)
        .foregroundStyle(Theme.Colors.textTertiary)
} else if let dueText {
    Text(dueText)
        .font(.caption)
        .foregroundStyle(isOverdue ? Theme.Colors.accentRed : Theme.Colors.textTertiary)
}
```

Replace with:

```swift
if let snoozeText {
    Text(snoozeText)
        .font(.caption)
        .foregroundStyle(Theme.Colors.textTertiary)
} else if entry.category == .habit, let cadence = entry.cadence {
    Text(cadence.displayName)
        .font(.caption)
        .foregroundStyle(Theme.Colors.textTertiary)
} else if let dueText {
    Text(dueText)
        .font(.caption)
        .foregroundStyle(isOverdue ? Theme.Colors.accentRed : Theme.Colors.textTertiary)
}
```

- [ ] **Step 4: Update opacity to 0.7 for snoozed**

Find the `.opacity(...)` modifier on the row (around line 624):

```swift
.opacity(entry.isDone ? 0.5 : 1.0)
```

Replace with:

```swift
.opacity(entry.isDone ? 0.5 : (isSnoozed ? 0.7 : 1.0))
```

- [ ] **Step 5: Build to verify**

Run: `make build`
Expected: build succeeds.

- [ ] **Step 6: Commit**

```bash
git add Murmur/Views/Home/AllEntriesView.swift
git commit -m "feat(all-view): snoozed row treatment in SmartListRow"
```

---

### Task 7: Swap swipe action to "Wake now" for snoozed entries

**Files:**
- Modify: `Murmur/Views/Home/DamHomeView.swift`
- Modify: `Murmur/Views/Home/ZonedFocusHomeView.swift`

- [ ] **Step 1: In `DamHomeView`, branch the snooze action on `entry.status`**

In `Murmur/Views/Home/DamHomeView.swift`, find the `swipeActions(for:)` (around line 200-215) and modify the snooze append:

```swift
if entry.status == .snoozed {
    actions.append(CardSwipeAction(
        icon: "sun.max.fill", label: "Wake",
        color: Theme.Colors.accentYellow
    ) { onAction(entry, .wake) })
} else {
    actions.append(CardSwipeAction(
        icon: "moon.zzz.fill", label: "Snooze",
        color: Theme.Colors.accentYellow
    ) { onAction(entry, .snooze(until: nil)) })
}
```

- [ ] **Step 2: Same for `ZonedFocusHomeView`**

In `Murmur/Views/Home/ZonedFocusHomeView.swift`, find `swipeActions(for:)` (around line 121-130) and apply the same branch:

```swift
private func swipeActions(for entry: Entry) -> [CardSwipeAction] {
    var actions: [CardSwipeAction] = [
        CardSwipeAction(icon: "checkmark.circle.fill", label: "Done", color: Theme.Colors.accentGreen) {
            onAction(entry, .complete)
        }
    ]
    if entry.status == .snoozed {
        actions.append(CardSwipeAction(icon: "sun.max.fill", label: "Wake", color: Theme.Colors.accentYellow) {
            onAction(entry, .wake)
        })
    } else {
        actions.append(CardSwipeAction(icon: "moon.zzz.fill", label: "Snooze", color: Theme.Colors.accentYellow) {
            onAction(entry, .snooze(until: nil))
        })
    }
    return actions
}
```

- [ ] **Step 3: Build to verify**

Run: `make build`
Expected: build succeeds.

- [ ] **Step 4: Commit**

```bash
git add Murmur/Views/Home/DamHomeView.swift Murmur/Views/Home/ZonedFocusHomeView.swift
git commit -m "feat(all-view): swap snooze swipe to wake on snoozed rows"
```

---

### Task 8: Manual verification on simulator

**Files:**
- None.

- [ ] **Step 1: Run on simulator**

Run: `make run`

- [ ] **Step 2: Verify toggle off (default)**

- The All tab shows the search bar with a `moon.zzz` (outline) button beside it, muted gray.
- No snoozed entries visible.

- [ ] **Step 3: Snooze an entry, verify it disappears**

Swipe-snooze any entry. It should disappear from the All tab.

- [ ] **Step 4: Verify toggle on**

Tap the moon button. Icon flips to `moon.zzz.fill` in yellow with a subtle glow. The snoozed entry reappears in its category section, marked with a moon icon (instead of a category dot) and showing "Until [time/date]" in the subtitle. Card opacity is reduced.

- [ ] **Step 5: Verify Wake swipe**

Swipe-left on the snoozed row. The snooze action label is now "Wake" with a sun icon. Tap it — the entry transitions to active, the moon icon swaps back to a category dot, and "Until X" disappears.

- [ ] **Step 6: Verify session reset**

Toggle on, kill the app, relaunch. Toggle should be off again.

- [ ] **Step 7: Verify analytics fire**

If running with analytics console output, confirm `all_view.snooze_filter_toggled` events appear with `enabled` and `snoozedCount` matching the toggle press.

- [ ] **Step 8: No commit (verification only)**

---

## Self-Review

**Spec coverage:**
- One toggle, session-only ✓ (Task 5)
- Snoozed mixed inline by category, sorted by priority/due ✓ (Task 5 — `combinedEntries` feeds existing `entriesByCategory` which already does priority/due sort)
- `RootView` exposes both `activeEntries` and `snoozedEntries` ✓ (Task 3)
- Toggle button beside search bar, `moon.zzz` / `moon.zzz.fill`, accentYellow on ✓ (Task 5)
- Snoozed row dot replaced with `moon.zzz.fill` in category color ✓ (Task 6)
- "Until X" subtitle replaces dueText for snoozed ✓ (Task 6)
- Opacity 0.7 for snoozed ✓ (Task 6)
- "Wake now" swipe action ✓ (Task 7)
- `EntryAction.wake` mirrors `unarchive` semantics ✓ (Task 1)
- `AllViewSnoozeFilterToggled` analytics event ✓ (Tasks 2 + 5)
- Auto-wake interaction: `wakeUpSnoozedEntries` keeps running; entries transition in place ✓ (no changes needed; merged source re-renders automatically)

**Placeholder scan:** All steps include actual code. No "TBD" or "similar to". ✓

**Type consistency:** `EntryAction.wake`, `AllViewSnoozeFilterToggled`, `snoozedEntries`, `showSnoozed`, `combinedEntries`, `isSnoozed`, `snoozeText` used consistently across tasks. ✓

---

## Rollback

If something breaks badly: `git revert` the per-task commits in reverse order. Each task is independently revertable except Tasks 4+5 which are bundled.
