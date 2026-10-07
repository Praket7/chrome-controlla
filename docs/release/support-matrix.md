# Release support matrix

This matrix separates build targets from verified product behavior. A successful host build is not live browser qualification.

| Area | Local evidence | Status |
|---|---|---|
| macOS arm64 package | Native package build and lifecycle checks | Host-verified |
| Linux x64 | GitHub Actions build, test, and package checks; run 37571371772 | CI-verified for commit `48994fc`; consumer install unverified |
| macOS x64 | GitHub Actions build, test, and package checks; run 37571371772 | CI-verified for commit `48994fc`; consumer install unverified |
| Windows x64 | GitHub Actions build, test, and package checks; run 37571371772 | CI-verified for commit `48994fc`; consumer install unverified |
| Chrome extension MV3 | Source fixtures and command allowlist check | Fixture-only; reload and live pairing required |
| Chrome direct CDP | Isolated fixture coverage | Fixture-only; separate browser qualification required |
| Private file artifacts | Unix-only temp-file fixture path; Windows fails closed | Windows artifact selection is not implemented until owner-only ACL behavior is qualified |
| Google Slides / Canva / CapCut Web | Generic route and planning contracts only | App acceptance not qualified |
| External MCP clients | Local configuration/guide checks | Client acceptance not qualified |
| Foreground, background, headless | Design and fixture coverage vary by route | Per-mode live qualification open |

Every release candidate must replace build-only rows with the exact artifact, OS/architecture, Chrome version, test profile, and evidence before claiming support.
