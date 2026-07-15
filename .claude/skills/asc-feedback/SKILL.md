---
name: asc-feedback
description: Fetch TestFlight beta feedback (screenshots, comments, crashes) for Jefe via the App Store Connect API. Use when someone says they left TestFlight feedback, or when debugging a field report from a beta build.
---

# ASC Beta Feedback — fetch

Pull TestFlight beta feedback submissions straight from the App Store Connect API
using the damsac team key. Fetching only — triage/issues/fixes are the caller's
business.

## Prerequisites

A team ASC API key (App Manager role suffices) seeded at `~/secrets/apple/`:

- `asc_key_id` — the key id (10 chars)
- `asc_issuer_id` — the issuer UUID
- `AuthKey.p8` — the private key

dam's machine has these; sac: ask dam for the trio once, seed the dir, never
commit them. The script reads them itself and never prints key material.

Python needs the `cryptography` package (`pip3 install cryptography` if missing).

## The tool

`asc-feedback.py` (this directory) — a minimal authenticated GET client for any
App Store Connect v1 endpoint:

```bash
python3 .claude/skills/asc-feedback/asc-feedback.py "<ASC API path>" [--raw]
```

## Recipe

1. **Resolve Jefe's app id** (bundle id `com.isaacwm.murmur`):

```bash
python3 asc-feedback.py "/v1/apps?filter[bundleId]=com.isaacwm.murmur" | jq -r '.data[0].id'
```

2. **List screenshot-feedback submissions** (comments + device metadata + image URLs):

```bash
python3 asc-feedback.py "/v1/apps/<APP_ID>/betaFeedbackScreenshotSubmissions?limit=50"
```

Useful fields per submission: `.attributes.comment`, `.attributes.createdDate`,
`.attributes.deviceModel` / `.osVersion`, `.attributes.screenshots[]` (each has a
signed `url` with an `expirationDate` — about a week), `.relationships.build.data.id`.

3. **List crash-feedback submissions**:

```bash
python3 asc-feedback.py "/v1/apps/<APP_ID>/betaFeedbackCrashSubmissions?limit=50"
```

4. **Download screenshots** (plain signed URLs, no auth header needed):

```bash
jq -r '.data[] | .id as $id | (.attributes.screenshots // [])[] | "\($id) \(.url)"' subs.json |
while read -r id url; do curl -sL "$url" -o "${id}.jpg"; done
```

5. **Resolve a build id to its build number** (feedback references builds by UUID):

```bash
python3 asc-feedback.py "/v1/builds/<BUILD_ID>" | jq -r '.data.attributes.version'
```

## Notes

- Screenshot URLs expire — download promptly.
- Works for any app on the team key (Athanor `com.damsac.athanor`, Weave
  `com.damsac.weave`) — just swap the bundle id.
- Precedent: dam's 9-submission build-44 batch became issues #220–#228 via this
  flow; screenshots for issues live on the `feedback-assets` orphan branch.
