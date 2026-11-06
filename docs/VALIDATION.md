# Validation record

Recorded during initial implementation, November 7, 2026 (Asia/Manila).

| Check | Result | Scope |
| --- | --- | --- |
| `npm ci` and production build | Passed | TypeScript and optimized, separately loaded office bundle |
| Rust core suite | 22 passed, 0 failed | Persistence, HTTP fixture requests, scoped tool writes, delegation, OAuth callback safety, cancellation, snapshots, and real pushes to temporary bare Git repositories |
| Playwright UI suite | 2 passed, 0 failed | Honest preview, agent selection/navigation, and native bridge connection-form contract |
| Tauri desktop compilation | Passed | Real Linux desktop shell in official Debian container with signed GTK/WebKit prerequisites |
| Tauri command-boundary test | 1 passed, 0 failed | Compiled/linked shell test rejects unsafe OAuth browser destinations |
| Windows process module cross-check | Passed | Windows API types compiled for the MSVC target; no Windows execution claimed |
| Visual inspection | Passed | Office screenshot in this directory; monochrome shell and distinguishable agent accents |

Browser tests use a mocked native bridge for form submission; they do not establish native provider execution. Core provider integration tests use real local HTTP fixtures rather than paid/provider account access. Loopback tests needed the executor's network permission because the default tool sandbox disallows listening.

Not verified in this machine: Windows installation/runtime, GTX1650/8GB performance, real Codex/Claude account execution, external gateway/model availability, or Google consent/Drive upload with a real account. Windows CI is configured to test and build an installer after push; an installer is available only after a successful CI run. Google Antigravity's direct adapter remains unimplemented because its official execution contract could not be verified under the current documentation-host network restriction.
