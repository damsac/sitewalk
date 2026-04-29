# All View Snooze Filter — Design

**Date:** 2026-04-29
**Author:** dam
**Status:** Approved (spec)

## Problem

The All tab on the home view shows only `status == .active` entries. Snoozed entries are completely hidden — there's no way to see "all my todos including the ones I snoozed" without going to the entry detail of a known item or waiting for the snooze to wake. Users hit a "where did that thing go?" moment.

## Goal

Add a single, simple visibility toggle to the All view: **show snoozed**. Off by default. When on, snoozed entries appear inline in their normal category sections, marked as snoozed.

Out of scope: cross-cutting filters (overdue, priority, due-today), completed/archived visibility, persistence across app launches.

## Behavior

- New `@State private var showSnoozed = false` in `AllEntriesView`. Session-only, resets to `false` each launch.
- `RootView` passes a new `snoozedEntries: [Entry]` alongside the existing `activeEntries`.
- Toggle off → render `activeEntries` only (today's behavior, no change).
- Toggle on → merge `activeEntries + snoozedEntries`, run the existing category grouping and sort. Snoozed entries land in their natural category section, sorted alongside active by `priority` → `dueDate` → `createdAt` desc.
- Search bar applies to the merged set when toggle is on.
- `wakeUpSnoozedEntries()` continues to run on foreground; if a snoozed entry wakes while toggle is on, it transitions to active in place (no flicker, just a re-sort).

## UI

### Toggle button

Sits beside the search bar at the top of `AllEntriesView`. 36×36 tap target, same vertical height as the search bar.

| State | SF Symbol     | Color                       | Treatment                |
|-------|---------------|-----------------------------|--------------------------|
| Off   | `moon.zzz`    | `Theme.Colors.textTertiary` | No fill                  |
| On    | `moon.zzz.fill` | `Theme.Colors.accentYellow` | Subtle glow (radius ~6) |

Tap = haptic light + toggle. Same icon family, same colors used by snooze swipe actions in `ZonedFocusHomeView` and `DamHomeView`.

### Snoozed row mark in `SmartListRow`

Three changes when rendering a snoozed entry:

1. **Replace the category dot** with `moon.zzz.fill` (12pt) in the entry's category color. Keeps category signal, marks state.
2. **Replace the dueText line** with "Until [snoozeUntil formatted]" — e.g., "Until tomorrow", "Until 9 AM", "Until Mar 8". Same typography (`Theme.Typography.caption`), `textTertiary` color.
3. **Card opacity 0.7** (lighter than the 0.5 used for completed — snoozed isn't done, just sleeping).

Habits don't typically get snoozed, but if they are, fall back to standard rendering (skip the habit-specific check-off button and apply the snooze treatment above).

### Tap and swipe

- Tap a snoozed row → opens entry detail (same as active).
- Swipe actions: existing snooze swipe action becomes "Wake now" on snoozed rows — sets `status = .active`, clears `snoozeUntil`, calls `NotificationService.shared.sync(...)`. (See `EntryAction.unarchive` for the closest existing pattern; we'll add an analogous flow without changing `EntryAction`.)

### Empty cases

- Toggle on but zero snoozed entries: no special empty state. The visible list is just `activeEntries`. Toggle button stays visible and tappable.
- Toggle on with search active and no matches: existing search empty state ("No results for X") works unchanged.

## Data flow

- `RootView.activeEntries` exists (line 638). Add `var snoozedEntries: [Entry]` next to it with the same exclusion guards (`pendingDeleteEntry`, `pendingRevealEntryIDs`) but filtered to `status == .snoozed`.
- `AllEntriesView` gains a new parameter: `snoozedEntries: [Entry]`. Combine internally based on `showSnoozed` state.
- The existing `entriesByCategory` computed property switches its source from `entries` to a `combinedEntries` computed property that conditionally merges based on `showSnoozed`.

## Analytics

New event in `Murmur/Services/MurmurEvents.swift`:

```swift
struct AllViewSnoozeFilterToggled: AnalyticsEvent {
    static let eventName = "all_view.snooze_filter_toggled"
    let enabled: Bool
    let snoozedCount: Int
}
```

Fired on every toggle press, before the state change applies (so `enabled` reflects the *new* state and `snoozedCount` reflects what was available at the moment of the toggle).

`snoozedCount` lets us answer: do people toggle when there's nothing to find? Signal of friction or curiosity vs. effective recall.

## Files touched

- `Murmur/Views/Home/AllEntriesView.swift` — toggle state, button, merged source, snooze mark in `SmartListRow`.
- `Murmur/Views/RootView.swift` — add `snoozedEntries`, pass to `AllEntriesView`.
- `Murmur/Services/MurmurEvents.swift` — new analytics event.
- `Murmur/Components/SwipeableCard.swift` or its callers — adjust snooze swipe action to "Wake now" when entry is snoozed.

## Out of scope

- Filtering for completed or archived (still in `ArchiveView`).
- Cross-cutting filters (overdue, priority, due-today).
- Toggle persistence across launches.
- Fixing `TodoListItem.swift:58` which uses `clock` instead of `moon.zzz` (separate inconsistency, flagged for follow-up).
- Surfacing the snooze toggle in the Focus tab — Focus is curated by the LLM and shouldn't include sleeping items.
