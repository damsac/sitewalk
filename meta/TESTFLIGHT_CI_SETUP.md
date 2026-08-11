# TestFlight CI Setup

One-time prerequisites for the `Release` workflow (`.github/workflows/release.yml`).
Once these are done, pushing to `main` ships an internal TestFlight build and
pushing a `v*` tag ships a build that's ready for external beta review.

---

## Rebuild-era update (2026-07-05)

`release.yml` was rewritten in place for the rebuilt app (Rust core + `apps/ios`
SwiftUI shell). Rewritten *in place* on purpose: `github.run_number` is
per-workflow-file, so keeping the same file keeps the build-number counter
monotonic vs the Era-I builds already on App Store Connect.

**Identical to Era-I (the proven signing blueprint, unchanged):**

- **Archive with automatic signing** — `CODE_SIGN_STYLE=Automatic` +
  `-allowProvisioningUpdates` + the ASC API key. Apple cloud-generates the
  Distribution cert and profile.
- **Export with manual signing** — `ExportOptions.plist` `signingStyle=manual`,
  explicit `provisioningProfiles` dict (`com.isaacwm.murmur` → `Murmur App
  Store`), cert imported via `Apple-Actions/import-codesign-certs@v3`, profile
  via `Apple-Actions/download-provisioning-profiles@v3`, **no**
  `-allowProvisioningUpdates` on export (cloud signing fails there under an App
  Manager key).
- **Bundle id `com.isaacwm.murmur`** and the **`Murmur App Store`** profile are
  reused (the TestFlight app + ASC build history are bound to that id/team
  `98GXNZ6NKZ`).
- Build number = `github.run_number`; a `v*` tag overrides `MARKETING_VERSION`.
- The `brew install … || true` / `brew link --overwrite` pattern; the XcodeGen
  #691 guard (`CODE_SIGN_IDENTITY: ""` in the spec, never set explicitly under
  Automatic).

**What changed for the rebuild:**

- **Ships the real Rust core.** A new step runs
  `apps/ios/build-ffi.sh --features whisper --device-only` on the macOS runner to
  produce the `MurmurCoreFFI` xcframework. No nix on the runner — `build-ffi.sh`
  now auto-detects nix and, when absent, runs against rustup's cargo + the system
  Xcode toolchain directly (the nix path is unchanged for local `nous` builds).
  `--device-only` builds only the `aarch64-apple-ios` slice (a TestFlight archive
  is device-only anyway), halving the whisper.cpp/Metal compile.
- **On-device model provisioning.** The workflow runs
  `apps/ios/fetch-whisper-model.sh` (the same script `./generate.sh` uses
  locally) to download the sha256-pinned `small.en` model (~190 MB, the
  default; `base.en` is a one-arg revert) from the ggerganov Hugging Face
  mirror into `apps/ios/Sources/Resources/`, cached via `actions/cache` keyed
  on the model name + the script's own hash (so a digest/model bump busts the
  cache automatically). The app-target `Sources` glob bundles it
  (`Bundle.main`); absent, live walks degrade to text-only.
- **Release xcodegen spec.** The Era-I `project.yml`/`Makefile` were removed in
  the re-unification; the release build now generates from
  `apps/ios/project-release.yml` (demo base + `MurmurCoreFFI` package + signed
  distribution overrides: bundle id, display name `Sitewalk`, version vars).
- **Dry-run capable.** Export uses `destination=export` (produces a signed `.ipa`
  artifact); the ASC upload is a **separate, conditional** `xcrun altool
  --upload-app` step. `workflow_dispatch` has an `upload` input (default
  `false`): a dry-run builds → signs → exports the `.ipa` but does **not**
  publish. Pushes to `main` / `v*` tags always upload.
- **No App Group, no entitlements, no analytics.** `apps/ios` uses none — it
  reads only `JEFE_PROXY_URL` + `JEFE_APP_SECRET` from `Info.plist`. So **step 1
  below (App Group registration) is NOT required** for the rebuilt app, and the
  profile is a plain App Store profile. `STUDIO_ANALYTICS_API_KEY_RELEASE` is no
  longer injected (there is no analytics SDK in the rebuild).
- **No Anthropic key in the build.** Shipped builds reach Anthropic only through
  the proxy (`services/proxy`), which holds the real `sk-ant-` key server-side.
  `Info.plist` has no key field, so there is nothing to extract from an IPA.

**Required secrets (rebuild):** `APPLE_TEAM_ID`, `ASC_API_KEY_ID`,
`ASC_API_ISSUER_ID`, `ASC_API_KEY_P8`, `APPLE_CERT_P12`, `APPLE_CERT_PASSWORD`,
`JEFE_APP_SECRET`. **Required variable:** `JEFE_PROXY_URL`.

**One-time steps you may still need before the first real upload:**

1. **Re-save / confirm the `Murmur App Store` profile matches the new app
   shape.** The rebuilt app has *fewer* capabilities than Era-I (no App Group).
   If the existing profile was created with the App Group entitlement, it should
   still work (it's a superset), but if provisioning errors on
   entitlement mismatch, edit the App ID `com.isaacwm.murmur` to remove the App
   Group capability (or leave it — the app simply won't request it) and
   regenerate/re-save the `Murmur App Store` profile. The
   `download-provisioning-profiles` step fetches the latest by bundle-id + type,
   so re-saving in the portal is all that's needed.
2. Everything in the Era-I steps below that concerns the **ASC API key (App
   Manager role)**, **auto-distribution to internal testers**, and the
   **repository secrets** still applies unchanged.

**Dry-run before publishing:** Actions → Release → *Run workflow* on this branch
with `upload=false`. It exercises the full chain — real-core FFI build, model
fetch, archive (automatic signing), export (manual signing) — and uploads the
signed `.ipa` as a build artifact **without** touching App Store Connect. Flip
`upload=true` (or merge to `main`) when the dry-run is green.

---

> **Note on bundle ID:** the bundle ID is `com.isaacwm.murmur` (registered
> under the damsac Apple Developer team). The `damsac` namespace is the
> GitHub org and the team in App Store Connect; the bundle ID prefix
> happens to be Isaac-namespaced because that's what was registered first.
> All instructions below use that bundle ID.

## 1. Register App Group capability on the App ID

The app uses the App Group `group.com.isaacwm.murmur.shared` for sharing data
between the app and any future extensions. Automatic provisioning will fail
unless this is registered as a capability on the App ID itself.

1. Open [Apple Developer → Identifiers](https://developer.apple.com/account/resources/identifiers/list).
2. Find the App ID `com.isaacwm.murmur` (create it if missing).
3. Edit → check **App Groups** under Capabilities → Configure → add
   `group.com.isaacwm.murmur.shared`.
4. Save.

## 2. Generate an App Store Connect API key

You'll need this for both signing (Apple cloud-generates the Distribution
cert and provisioning profile on demand) and the TestFlight upload.

1. Open [App Store Connect → Users and Access → Integrations → App Store
   Connect API](https://appstoreconnect.apple.com/access/integrations/api).
2. Generate a new key. **Role: App Manager.** (Developer cannot sign for
   distribution.)
3. Download the `.p8` file immediately — Apple will not let you download it
   again. Note the **Key ID** and the **Issuer ID** shown on the page.

## 3. Configure auto-distribution to internal testers

1. Open the Murmur app in App Store Connect.
2. TestFlight tab → Internal Testing → your group.
3. Enable **automatic distribution** so internal testers get every uploaded
   build immediately.

External testers stay manual: after a tag-triggered build lands in TestFlight,
go to the build, add it to the external group, and submit for Beta App Review.

## 4. Add GitHub repository secrets

[Settings → Secrets and variables → Actions](https://github.com/damsac/Murmur/settings/secrets/actions)
on the repository. Add each as a "Repository secret":

| Secret | Value |
|---|---|
| `APPLE_TEAM_ID` | 10-character team ID (e.g., `ABCD123456`). Find at [Apple Developer → Membership](https://developer.apple.com/account#MembershipDetailsCard). |
| `ASC_API_KEY_ID` | Key ID from step 2 (e.g., `2X9YPDMS47`). |
| `ASC_API_ISSUER_ID` | Issuer UUID from step 2. |
| `ASC_API_KEY_P8` | **Full contents** of the `.p8` file from step 2. Paste the file as text, including the `-----BEGIN PRIVATE KEY-----` / `-----END PRIVATE KEY-----` lines. Do not base64-encode. |
| `JEFE_APP_SECRET` | Shared secret the app presents to the LLM proxy. Must match the `APP_SECRET` set on the Cloudflare worker (`wrangler secret put APP_SECRET`). |
| `JEFE_PROXY_URL` (a **variable**, not a secret) | The deployed worker URL, e.g. `https://jefe-proxy.damsac-app.workers.dev`. |
| `STUDIO_ANALYTICS_API_KEY_RELEASE` | `sk_murmur` (the Release-config Studio Analytics key). |

## 5. Verify

Push a no-op commit to `main`. Watch the Release workflow run in
[GitHub Actions](https://github.com/damsac/Murmur/actions/workflows/release.yml).
A new build should appear in App Store Connect → TestFlight within a few
minutes of the workflow completing. Internal testers get it automatically.

To ship to externals, push a tag:

```bash
git tag v1.0.1
git push origin v1.0.1
```

The build will land in TestFlight with `MARKETING_VERSION=1.0.1`. Open it in
ASC, add the external group, and submit for Beta App Review.

## Troubleshooting

- **"No profiles for ... were found"** — App Group capability not registered
  on the App ID (step 1) or the API key role is too low (step 2).
- **"Invalid version number"** — TestFlight rejects a build whose
  `MARKETING_VERSION + CURRENT_PROJECT_VERSION` already exists. Build numbers
  come from `github.run_number` so collisions are impossible from CI; if you
  see this, you likely uploaded a colliding build manually from a laptop.
- **"Authentication failed"** on `xcodebuild` — the `.p8` was pasted with
  trailing whitespace stripped or BEGIN/END lines missing. Re-paste the
  entire file verbatim.
- **Workflow fails on tag push with "Tag does not match v<major>.<minor>.<patch>"** —
  use a strict semver tag (`v1.0.0`, not `v1.0` or `release-1.0.0`).
