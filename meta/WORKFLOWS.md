# Workflows

How dam and sac work together on Sitewalk. The day-to-day practices.

---

## The cycle

```
/start → pick work → build → /ship → review → merge → reconcile
```

### 1. Start a session (`/start`)

- Sync with main
- Read the other person's STATE.md — know their headspace
- Check CANON.md for new decisions
- Check ROADMAP.md for priorities
- Look at open PRs that need review
- Pick work from the board or propose something new

### 2. Build

- Work on your own branch — commit freely
- Each person works with their own Claude instance
- Claude's reasoning is the valuable artifact — not just the code it produces
- Build with `direnv allow` (Nix dev shell: Rust toolchain, xcodegen, ...);
  see `CLAUDE.md` for the full Rust + iOS command table — not duplicated here

### 3. Ship (`/ship`)

- Update your STATE.md with decisions, open questions, needs
- Branch off for the PR: `git checkout -b pr/dam/<pr-name>` (or `pr/sac/<pr-name>`)
- Open PR from `pr/<you>/<pr-name>` → `main` with a **Thinking** section
- CI gates every PR automatically (nix Rust build+test+clippy, iOS demo build) —
  don't merge until it's green
- Update ROADMAP.md if priorities changed
- Propose CANON.md additions if decisions were made
- Merges land on `main` via PR only (no direct pushes)

### After a PR merges

Rebase your working branch onto main to stay clean:
```
git checkout main && git pull
git checkout <your-branch> && git rebase main
```

This keeps your next PR's diff clean — no duplicate commits from already-merged work.

### 4. Review

The reviewer's job is to review **thinking**, not code.

Read in this order:
1. **Thinking section** — do I agree with the reasoning?
2. **STATE.md diff** — do I understand their current headspace? Any questions for me?
3. **Canon candidates** — do I agree these should be canonical?
4. **Code** — does the implementation match the thinking?

If the thinking is wrong, the code doesn't matter. Push back on the thinking.
If the thinking is sound but the code is off, that's a smaller conversation.

Sac reviews dam's PRs; dam reviews sac's PRs. Linear thinking review — see
`meta/RECONCILIATION.md` for the full protocol.

### 5. Reconcile

After merging:
- New canon entries take effect immediately
- Both STATE files should be consistent with reality
- ROADMAP reflects what just shipped and what's next

## Who does what

**dam** — harness / murmur-core / STT / FFI: `crates/harness`, `crates/murmur-core`,
`crates/stt`, `crates/ffi`. **sac** — renderers / component library / visual
direction: `apps/ios/` (SwiftUI shell). Both touch whatever needs touching — these
are centers of gravity, not walls.

## Communication patterns

### Async (default)
- PRs are the primary communication channel
- STATE.md is the "here's where my head is at" signal
- CANON.md is the "here's what we've agreed on" record
- GitHub issues for new work items

### Sync (when needed)
- When a CANON decision is contentious (see RECONCILIATION.md)
- When STATE files show conflicting assumptions
- When both are touching the same area of the codebase

## What goes where

| Artifact | Location | When to update |
|----------|----------|---------------|
| Shared decisions | `meta/CANON.md` | When both agree on something |
| Priorities | `meta/ROADMAP.md` | When work ships or priorities shift |
| Your current state | `meta/<you>/STATE.md` | Every PR |
| Your process | `meta/<you>/PROCESS.md` | When your workflow changes |
| Plans (specs before building) | `docs/plans/`, `docs/superpowers/plans/` | Before building |
| Research / design explorations | `docs/research/`, `docs/brainstorms/` | When exploring |
| How we work | `meta/WORKFLOWS.md` (this file) | When practices evolve |
| PR reconciliation protocol | `meta/RECONCILIATION.md` | Rarely (it's the protocol) |

Build commands, architecture, and dev-shell detail live in `CLAUDE.md` (repo root)
and `apps/ios/README.md` — don't duplicate them here; link to them instead.
