# TestFlight Readiness Audit

**Date:** 2026-03-03
**Branch:** `dam`
**Auditor:** Claude (automated)

> **HISTORICAL — Era-I snapshot.** Section 2 ("API Key Injection") and blocking
> item 2 describe the `PPQ_API_KEY` → `Info.plist` chain and `APIKeyProvider.swift`,
> neither of which exists any more. **Closed:** shipped builds now reach Anthropic
> only through the Cloudflare proxy (`services/proxy`), and `Info.plist` carries no
> Anthropic key field, so there is nothing to extract from an IPA. Kept as a record
> of what the audit found at the time.

---

## 1. Signing Configuration

| Item | Status | Detail |
|------|--------|--------|
| `CODE_SIGN_STYLE` | OK | `Automatic` — fine for TestFlight |
| `CODE_SIGN_IDENTITY` | **BLOCKING** | Set to `"Apple Development"` only. Archive builds require `"Apple Distribution"`. Automatic signing *may* handle this, but there's no explicit release config. |
| `DEVELOPMENT_TEAM` | OK (per-dev) | Injected via `project.local.yml`. Will work if set correctly. |
| Archive configuration | **NEEDS ATTENTION** | No `configs:` block in `project.yml`. XcodeGen will generate default Debug/Release, but there's no explicit `Release` signing override. Verify Xcode resolves this automatically via Automatic Signing. |
| Scheme archive config | **MISSING** | The `scheme:` block only has `storeKitConfiguration` and `testTargets`. No `archive:` configuration (e.g., `buildConfiguration: Release`). XcodeGen defaults to Release for archive, which is likely fine, but it's implicit. |

## 2. API Key Injection

| Item | Status | Detail |
|------|--------|--------|
| Build-time injection | OK | `PPQ_API_KEY` → `$(PPQ_API_KEY)` in `project.yml` → `PPQAPIKey` in Info.plist → `Bundle.main.object(forInfoDictionaryKey:)` in `APIKeyProvider.swift`. This chain works for archive builds. |
| Empty key handling | OK | `APIKeyProvider.ppqAPIKey` returns `nil` if empty. `AppState.configurePipeline()` prints a warning and sets `pipelineError`. |
| Key in archive | **BLOCKING** | The API key value comes from `project.local.yml` which is gitignored and per-developer. For TestFlight, whoever archives must have a valid `PPQ_API_KEY` in their `project.local.yml`. There's no CI/build server config for this. The key will be baked into the binary (visible in Info.plist). |
| Key security | **NEEDS ATTENTION** | API key is embedded in plain text in Info.plist inside the app bundle. Any TestFlight/App Store user can extract it. Consider server-side proxying or obfuscation. |

## 3. Debug-Only Code

| Item | Status | Detail |
|------|--------|--------|
| `isDevMode` property | OK | Properly gated: `#if DEBUG` → `true`, `#else` → `false`. Release builds default to `false`. |
| Dev mode floating button | OK | `RootView.swift:37` — wrapped in `#if DEBUG`. Won't compile into release. |
| Dev mode sheet | OK | `RootView.swift:184` — wrapped in `#if DEBUG`. |
| **DevModeActivator** | **BLOCKING** | `DevModeActivator.swift` is **NOT** gated by `#if DEBUG`. It compiles into release. `HomeView.swift:78` calls `.devModeActivator()` without any conditional. In release, 5-tapping opens `DevModeView` and sets `isDevMode = true`. Users can toggle onboarding, focus, and regenerate daily focus in production. |
| **DevModeView** | **BLOCKING** | `DevModeView.swift` is also not gated. It's included in the build (not in `project.yml` excludes). Only `DevScreen.swift`, `DevComponentGallery.swift`, `DevComponent.swift` are excluded. |
| `print()` statements | **NEEDS ATTENTION** | ~80+ `print()` calls across the codebase. Most are in `#Preview` blocks (harmless), but several are in production code paths: `NotificationService.swift:23`, `AgentActionExecutor.swift:75,228`, `AppState.swift:58`, `OnboardingFlowView.swift:116`, `EntryDetailView.swift:274`, `RootView.swift:473`, `PersistenceConfig.swift:44`, `Entry.swift:328`. These will appear in device console logs. Not blocking but noisy. |

## 4. App Version / Build Number

| Item | Status | Detail |
|------|--------|--------|
| `MARKETING_VERSION` | OK | `"1.0.0"` — valid for initial release |
| `CURRENT_PROJECT_VERSION` | **NEEDS ATTENTION** | `"1"` — fine for first upload, but must be incremented for each subsequent TestFlight upload. No automation for this. |
| Version injection | OK | Both use `$(MARKETING_VERSION)` / `$(CURRENT_PROJECT_VERSION)` in Info.plist correctly. |

## 5. Entitlements & Capabilities

| Item | Status | Detail |
|------|--------|--------|
| App Groups | OK | `com.apple.security.application-groups` with `$(APP_GROUP_IDENTIFIER)`. Must match App Store Connect config. |
| Microphone | OK | `NSMicrophoneUsageDescription` set with good copy. |
| Speech Recognition | OK | `NSSpeechRecognitionUsageDescription` set with good copy. |
| Push Notifications | OK (for now) | App uses `UNUserNotificationCenter` for local notifications (reminders/due dates). No `aps-environment` entitlement needed for local-only. Will need it if remote push is added later. |
| Export Compliance | **BLOCKING** | No `ITSAppUsesNonExemptEncryption` key in Info.plist. TestFlight will prompt you manually on every upload. Add `ITSAppUsesNonExemptEncryption: false` to Info.plist properties. The app uses HTTPS (standard) but no custom encryption. |

## 6. Assets

| Item | Status | Detail |
|------|--------|--------|
| App Icon | OK | `AppIcon.appiconset` has a valid 1024x1024 PNG (RGBA, 460KB). `Contents.json` references it correctly for universal iOS. |
| Launch Screen | OK | Uses `UILaunchScreen` dict in Info.plist with `LaunchBackground` color. Color asset exists at `Assets.xcassets/LaunchBackground.colorset/`. No storyboard needed. |
| StoreKit config | OK | `Murmur.storekit` exists with consumable credit products. This is for testing only — real products must be configured in App Store Connect. |

## 7. TODO/FIXME/HACK Comments

| Item | Status | Detail |
|------|--------|--------|
| App target | OK | Only match is in `MockDataService.swift` which is excluded from the build. |
| MurmurCore | OK | No matches. |

---

## Summary

### Blocking (must fix before TestFlight)

1. **DevModeActivator ships in release** — `DevModeActivator.swift` and `DevModeView.swift` are not `#if DEBUG` gated and not excluded from the build. Users can 5-tap to open dev tools in production.
2. **API key baked in plain text** — `PPQ_API_KEY` is embedded in Info.plist. Extractable by anyone with the IPA. This is a security/cost risk if the key has no rate limiting.
3. **Missing `ITSAppUsesNonExemptEncryption`** — Without this, every TestFlight upload triggers a manual export compliance prompt.

### Needs Attention (should fix, not strictly blocking)

4. **No explicit release signing identity** — relying entirely on Xcode Automatic Signing to resolve `Apple Development` → `Apple Distribution` for archive. Usually works, but untested.
5. **Build number not automated** — `CURRENT_PROJECT_VERSION: "1"` must be manually bumped for each TestFlight upload.
6. **~10 `print()` calls in production code paths** — will leak to device console. Consider `os.log` or `#if DEBUG` wrapping.
7. **StoreKit products** — local `.storekit` file exists for testing, but real products must be set up in App Store Connect before credit purchasing works in TestFlight.

### Ready

- App icon, launch screen, entitlements, usage descriptions, version string, project structure, build chain, notification permissions, API key injection mechanism.
