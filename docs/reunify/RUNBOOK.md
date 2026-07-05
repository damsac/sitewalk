# Phase 2 runbook — re-unify sitewalk into Murmur

This is the exact command sequence for folding `damsac/sitewalk` back into
`damsac/Murmur` via an unrelated-histories merge. **Do not run any of this until
the in-flight sitewalk work has landed** (sitewalk PR #1 merged, issue #2
follow-ups resolved or explicitly deferred, harness patches PR'd). Phase 1 (PR
#152, the pivot chapter) is already on Murmur `main`.

Principle: **carry history over, never copy-import.** The Rust rebuild's
commit-by-commit history is the story — it must arrive via
`git merge --allow-unrelated-histories`, not by copying files into a commit.

Run everything from a clean checkout of `damsac/Murmur` on an up-to-date `main`.
Work on a branch (`pr/dam/reunify-merge`) and land via PR — `main` is protected.

---

## 0. Preconditions (verify first)

```bash
cd /Users/claude/Murmur
git switch main && git pull --ff-only origin main
git status --porcelain            # MUST be clean except known untracked secrets
# Never stage these — signing secrets at repo root:
#   Certificates.p12   distribution.cer   docs/legal/   (and Packages/MurmurCore/Sources/ScenarioRunner/)
# Stage every file explicitly by path in this runbook. NEVER `git add -A` / `git add .`.
gh repo view damsac/sitewalk --json isArchived    # must be false (not yet archived)
```

Confirm sitewalk is at the intended tip:

```bash
git ls-remote git@github.com:damsac/sitewalk.git refs/heads/main
```

---

## 1. Pivot commit — retire the Swift app from the tree (FIRST, so the merge is clean)

Removing the Swift app before the merge means sitewalk's Rust workspace lands
without path collisions against `Murmur/` and `Packages/MurmurCore/`.

```bash
git switch -c pr/dam/reunify-merge

# Remove the Swift app tree (history preserves it):
git rm -r Murmur/ Packages/MurmurCore/
# Swift-era build/config that no longer applies at the root:
git rm project.yml project.local.yml.template Makefile 2>/dev/null || true
# (Leave meta/, docs/, flake.nix, .gitignore — sitewalk's versions win in step 3.)

git commit -m "pivot: retire the Swift app from the tree (Era I preserved in history)"
```

Record this SHA in `docs/HISTORY.md` (replace `<PIVOT_COMMIT_SHA>`):

```bash
git rev-parse HEAD
```

> Note the exact set of root files to remove against the live tree at run time —
> `git rm` only what actually exists. Do NOT remove the untracked secrets or
> `docs/legal/`.

---

## 2. Add sitewalk as a remote and fetch its history

```bash
git remote add sitewalk git@github.com:damsac/sitewalk.git
git fetch sitewalk main
```

---

## 3. Merge with unrelated histories

```bash
git merge --allow-unrelated-histories sitewalk/main \
  -m "reunify: merge damsac/sitewalk history into Murmur (Rust rebuild rejoins)"
```

**Expected collisions** (both repos have these paths). sitewalk's versions are
current — **sitewalk wins**:

| Path | Resolution |
|------|-----------|
| `meta/` | sitewalk version wins (current STATE/ROADMAP/CANON of the rebuild) |
| `docs/` | **merge by hand** — Murmur has the pivot-chapter docs (HISTORY.md, superpowers/, this runbook) that sitewalk lacks; keep BOTH sides. Take sitewalk's rebuild docs, keep Murmur's `docs/HISTORY.md`, `docs/reunify/`, `docs/superpowers/`. |
| `.gitignore` | sitewalk version wins (Rust-oriented); re-add any Murmur-only ignores still needed |
| `flake.nix` | sitewalk version wins (Rust dev shell) |

Resolve, then stage each resolved path **explicitly**:

```bash
git checkout --theirs meta/ .gitignore flake.nix     # sitewalk = "theirs"
git add meta/ .gitignore flake.nix
# docs/: reconcile by hand (keep both eras' docs), then:
git add docs/
git commit --no-edit    # completes the merge commit
```

Record the merge SHA in `docs/HISTORY.md` (replace `<SITEWALK_MERGE_COMMIT_SHA>`).
Its second parent (`^2`) is the sitewalk history root:

```bash
git rev-parse HEAD
git log --oneline HEAD^2 | head    # sanity: sitewalk's rebuild commits are present
```

---

## 4. Swap the README and finalize docs

```bash
git mv docs/reunify/README.next.md README.md   # promote the drafted README
# Edit README.md: remove the DRAFT banner, confirm crate/app paths against the tree.
git add README.md docs/HISTORY.md               # HISTORY.md now has real SHAs
git commit -m "docs: post-reunification README + fill history SHAs"
```

---

## 5. Verify, push, PR

```bash
cargo build && cargo test && cargo clippy       # Rust workspace at root now
git log --graph --oneline -20                   # both histories visible
git push origin pr/dam/reunify-merge
gh pr create --base main --head pr/dam/reunify-merge \
  --title "Re-unify: merge sitewalk history into Murmur (Phase 2)" \
  --body "..."   # include a Thinking section (house rule)
# Merge with a MERGE COMMIT (not squash) — preserves both histories.
gh pr merge --merge
```

---

## 6. Migrate sitewalk issue #2

sitewalk issue #2 = "PR #1 review follow-ups: seam hygiene + 4 state-transition
bugs." Re-file on Murmur (gh has no cross-repo transfer for archived targets;
recreate + link):

```bash
gh issue view 2 --repo damsac/sitewalk --json title,body,labels > /tmp/sw-issue-2.json
gh issue create --repo damsac/Murmur \
  --title "PR #1 review follow-ups: seam hygiene + 4 state-transition bugs (migrated from sitewalk#2)" \
  --body "Migrated from damsac/sitewalk#2 ahead of archiving.\n\n<original body>"
# Comment on sitewalk#2 with the new Murmur issue link, then close sitewalk#2.
gh issue comment 2 --repo damsac/sitewalk --body "Migrated to damsac/Murmur#<N>. Closing; sitewalk is being archived."
gh issue close 2 --repo damsac/sitewalk
```

---

## 7. Pointer README on sitewalk, then archive

Push a pointer README to sitewalk **before** archiving (archived repos are
read-only):

```bash
# In a sitewalk checkout, on main:
cat > README.md <<'EOF'
# sitewalk → merged into damsac/Murmur

This repo held the Rust rebuild of **Murmur** (field-work voice agent). As of
2026-07-04 its full history has been merged back into **[damsac/Murmur](https://github.com/damsac/Murmur)**
via `git merge --allow-unrelated-histories` — nothing was lost. Development
continues there.

This repository is **archived** (read-only). Its commit history is preserved and
also browsable inside damsac/Murmur (see `docs/HISTORY.md` there).
EOF
git add README.md
git commit -m "docs: archive pointer — history merged into damsac/Murmur"
git push origin main

# Then archive:
gh repo archive damsac/sitewalk --yes
```

---

## Post-conditions checklist

- [ ] Swift app removed from Murmur tree; recoverable via `git show <PIVOT_COMMIT_SHA>^:…`
- [ ] `git log HEAD^2` on the merge commit shows sitewalk's rebuild history
- [ ] `docs/HISTORY.md` placeholders replaced with real SHAs
- [ ] `README.md` describes the field-work product + Rust workspace
- [ ] Murmur issue mirrors sitewalk#2; sitewalk#2 closed with a link
- [ ] sitewalk has a pointer README and is archived
- [ ] Signing secrets (`Certificates.p12`, `distribution.cer`, `docs/legal/`) never committed
