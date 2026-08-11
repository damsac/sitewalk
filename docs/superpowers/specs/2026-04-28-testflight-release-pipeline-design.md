# TestFlight Release Pipeline — Design

**Date:** 2026-04-28
**Status:** Approved — **amended:** the `PPQ_API_KEY` secret described below is
gone. Shipped builds reach Anthropic only through the Cloudflare proxy
(`services/proxy`); CI injects `JEFE_PROXY_URL` + `JEFE_APP_SECRET` and no
Anthropic key enters the build. See `meta/TESTFLIGHT_CI_SETUP.md` for the
current secret list.
**Replaces:** scaffold in PR #126 (`pr/dam/ci-archive`)

## Goal

Push code, get a TestFlight build. No App Store deployment yet.

Two release lanes:

- **Main push** → internal TestFlight (continuous; team dogfoods every commit)
- **Tag push** (`v*`) → external TestFlight (curated; gets submitted for Apple Beta Review and assigned to external testers)

Both lanes upload to the same TestFlight via the same job. The internal-vs-external split is configured **once in App Store Connect** (auto-distribute new builds to the internal group). External assignment + beta review submission stays a manual click in ASC after a tag build lands.

## Non-goals

- App Store production releases.
- Automated external beta review submission. (Future improvement once the pipeline is stable.)
- Manual signing with imported `.p12` cert + `.mobileprovision` profile. (Replaced by ASC API key + automatic signing.)
- Per-PR build pipeline. (Existing `ci.yml` runs tests on PR; this workflow only fires on `main` and tags.)

## Trigger model

Single workflow file `.github/workflows/release.yml` (renamed from the existing `ci-archive.yml`):

```yaml
on:
  push:
    branches: [main]
    tags: ['v*']
```

Inside the job, branch on `github.ref`:

- `refs/heads/main` → main path
- `refs/tags/v*` → tag path

The two paths share all build, sign, and upload steps. The only differences:

- `MARKETING_VERSION` derivation
- Optional ASC build description / "What to test" notes (out of scope for v1)

## Versioning

### Build number

`CURRENT_PROJECT_VERSION = ${{ github.run_number }}` for both paths.

`github.run_number` is monotonic across every run of this workflow regardless of trigger, so no two builds can collide on `(MARKETING_VERSION, CURRENT_PROJECT_VERSION)`. TestFlight rejects duplicates; this scheme makes that impossible.

### Marketing version

- **Main push** → reads `MARKETING_VERSION` from `project.yml` (currently `1.0.0`). Bump in source when you want main builds to track a new version.
- **Tag push** (`v1.2.3`) → workflow parses tag, strips leading `v`, exports `MARKETING_VERSION=1.2.3` at xcodebuild invocation time. `project.yml` is **not** edited.

Tags are independent of `project.yml`. You do not need to keep them in sync.

Tag pattern is `v*` for the trigger filter; the parser uses a stricter regex (`^v\d+\.\d+\.\d+$`) and fails the build if the tag doesn't match. (Pre-release tags like `v1.2.3-beta.1` are out of scope; we'll add them only when needed.)

## Signing & upload

### Approach

App Store Connect API key with **automatic signing** via `-allowProvisioningUpdates`. Apple cloud-manages the Distribution cert and the App Store provisioning profile. The same API key handles the TestFlight upload via `altool`.

### Secrets

Stored in GitHub Settings → Secrets and variables → Actions:

| Secret | Purpose |
|---|---|
| `APPLE_TEAM_ID` | 10-character team identifier |
| `ASC_API_KEY_ID` | Key ID from App Store Connect |
| `ASC_API_ISSUER_ID` | Issuer UUID for the team |
| `ASC_API_KEY_P8` | Full contents of the `AuthKey_*.p8` file (text, not base64) |
| `PPQ_API_KEY` | Baked into Info.plist as `PPQAPIKey` |
| `STUDIO_ANALYTICS_API_KEY_RELEASE` | `sk_murmur`, used by Release config only |

### Build flow

1. **Generate `project.local.yml` from secrets.** Sets `DEVELOPMENT_TEAM`, bundle IDs, app group, `PPQ_API_KEY`, and the `Release` config's `STUDIO_ANALYTICS_ENDPOINT` + `STUDIO_ANALYTICS_API_KEY`. `Debug` config left empty (CI doesn't run Debug builds for release).
2. **`make generate`** — XcodeGen produces `Murmur.xcodeproj`.
3. **Materialize the API key** to a temp `.p8` file from `ASC_API_KEY_P8`.
4. **`xcodebuild archive`** with:
   - `-project Murmur.xcodeproj -scheme Murmur`
   - `-destination 'generic/platform=iOS'`
   - `-archivePath $RUNNER_TEMP/Murmur.xcarchive`
   - `-allowProvisioningUpdates`
   - `-authenticationKeyID $ASC_API_KEY_ID`
   - `-authenticationKeyIssuerID $ASC_API_ISSUER_ID`
   - `-authenticationKeyPath $RUNNER_TEMP/AuthKey.p8`
   - `CODE_SIGN_STYLE=Automatic`
   - `CODE_SIGN_IDENTITY="Apple Distribution"`
   - `DEVELOPMENT_TEAM=$APPLE_TEAM_ID`
   - `MARKETING_VERSION=$VERSION` (parsed from tag, or omitted on main path)
   - `CURRENT_PROJECT_VERSION=${{ github.run_number }}`
   - Piped through `xcbeautify`
5. **`xcodebuild -exportArchive`** with an `ExportOptions.plist` containing:
   ```xml
   method: app-store-connect
   destination: upload
   signingStyle: automatic
   teamID: $APPLE_TEAM_ID
   ```
   Note: with `destination: upload`, `xcodebuild` itself uploads to TestFlight using the supplied API key — no separate `altool` step needed. (Confirmed in implementation; if `destination: upload` proves flaky, fall back to `destination: export` + explicit `xcrun altool --upload-app`.)
6. **Upload `.xcarchive` as Actions artifact** for 30-day retention. Useful for replay/debugging signing failures.

### Project changes required outside CI

`project.yml` currently has:

```yaml
CODE_SIGN_STYLE: Automatic
CODE_SIGN_IDENTITY: "Apple Development"
```

The `CODE_SIGN_IDENTITY` value is fine for local Debug builds but is overridden in CI by xcodebuild flags. **No edit needed to `project.yml`.**

### What gets removed from PR #126's scaffold

- Conditional "build unsigned if no secrets" branch (the workflow now requires real secrets and hard-fails without them — cleaner than silent fallback)
- `.p12` certificate import step
- `.mobileprovision` profile import step
- Keychain creation, unlock, and cleanup steps
- `xcrun altool --upload-app` separate step (folded into `xcodebuild -exportArchive` with `destination: upload`)

The workflow gets noticeably shorter.

## Prerequisites (one-time, outside CI)

These must be done before the first CI run will succeed:

1. **Apple Developer Portal** — register `group.com.damsac.murmur.shared` as an App Group capability on the App ID `com.damsac.murmur`. Without this, automatic provisioning fails because the requested entitlements don't match what the App ID allows. (Already flagged in `meta/TESTFLIGHT_CHECKLIST.md` item #14.)
2. **App Store Connect** — generate an API key with **App Manager** role (Developer role cannot sign for distribution). Save the `.p8` file immediately — Apple does not allow re-downloading it.
3. **App Store Connect** — configure the internal test group to "auto-distribute new builds" so internal testers get every uploaded build automatically.
4. **GitHub** — add the six secrets listed above to the repository.

A `meta/TESTFLIGHT_CI_SETUP.md` doc captures these steps so they're not lost.

## Failure modes & responses

| Failure | Likely cause | Response |
|---|---|---|
| `xcodebuild archive` fails with provisioning error | App Group not registered on App ID | Register in Developer Portal (prereq #1) |
| Upload rejected: duplicate build number | Should be impossible with `github.run_number`, but if a re-run somehow collides | Manually re-trigger the workflow with `workflow_dispatch` (not in scope yet) or push a no-op commit |
| Upload rejected: marketing version regression | Tag pushed with version lower than what's in TestFlight | Push a higher tag; old tags can't be re-released |
| ASC API key revoked or expired | Apple revokes keys after team-level changes | Generate a new key, update `ASC_API_KEY_P8`/`ASC_API_KEY_ID` |
| Tag doesn't match `^v\d+\.\d+\.\d+$` | Typo in tag | Workflow fails fast in the version-parse step before any build work; delete tag and re-push |

## Test plan

- Set the six secrets in GitHub.
- Run prereqs #1–3 (App Group, ASC API key, internal auto-distribute).
- Push a no-op commit to `main` → verify a build appears in TestFlight, build number = run number, marketing version = `1.0.0` (from `project.yml`).
- Push a tag `v1.0.1` → verify a second build appears in TestFlight, build number > previous, marketing version = `1.0.1`.
- In ASC, manually submit the tag build for external beta review and assign to external group → verify the external tester flow.

## Open questions

None — design is complete enough to write the implementation plan.

## Files

- New / modified:
  - `.github/workflows/release.yml` (rewritten from `ci-archive.yml`)
  - `meta/TESTFLIGHT_CI_SETUP.md` (new — captures the one-time prereqs)
- Deleted:
  - `.github/workflows/ci-archive.yml` (renamed to `release.yml`)
- Untouched:
  - `project.yml` — no edits needed; xcodebuild flags override signing for CI
  - Existing `ci.yml`, `ci-lint.yml`, `pages.yml` — unchanged
