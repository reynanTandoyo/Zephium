# Project structure

A map of the Zephium repository: what each part does and where to find it.
It summarises the code as of upstream commit `0a53d5c4e` (2026-11). The
authoritative sources are [docs/architecture.md](docs/architecture.md),
[docs/frontend.md](docs/frontend.md), [docs/work.md](docs/work.md) and
[CONTRIBUTING.md](CONTRIBUTING.md). If the code and this file disagree, the
code is right; fix this file.

## 1. What Zephium is

An open-source desktop browser (Windows and macOS; **Linux is unsupported**)
written in Rust. It uses the operating system's own web engine: **WebView2**
on Windows and **WebKit** on macOS, not Chromium or Firefox. The interface is
**Svelte 5** running inside **Tauri 2**. Main features:

- Browsing: tabs, spaces, split view, bookmarks, history, downloads, notes,
  tasks, time and focus tools.
- Native ad and tracker blocking, and Chrome extensions.
- **Ask Zephium**: a sidebar assistant that reads the current page.
- **Work**: an agentic canvas. Helpers browse live pages, use MCP connectors,
  folders and shell, and lay results out on the canvas. It works with your
  own model keys (Anthropic, OpenAI, Gemini, …) or local models. The
  fork removed the Zephium account and the hosted "Zephium AI" provider.

| Fact | Value |
| --- | --- |
| License | MPL-2.0 (blocker seed lists: CC-BY-SA-3.0 / GPL-3.0) |
| Rust | `1.95.0` (`rust-toolchain.toml`), edition 2021 |
| Node / pnpm | `24.18.x` / `11.17.0` |
| Size | about **823k** lines of Rust in `crates/` (1,153 files), about **60k** in `desktop/src`, about **230k** lines of TS/Svelte in `frame/src` |

## 2. Top-level layout

| Path | Contents |
| --- | --- |
| `crates/` | 28 Rust library crates: the browser domain, engine, storage, blocker, agents, Work |
| `desktop/` | Tauri app crate `zephium-desktop`, the **composition root**: wires services, IPC, plugins, windows |
| `frame/` | Svelte/TypeScript interface (browser window, launcher panel, onboarding) |
| `xtask/` | `cargo xtask …` developer and CI tooling: boundary checks, blocker seed, signing, `ci` aggregate |
| `scripts/` | CI, release, QA and blocker tooling scripts (Node, PowerShell, shell, Python, Swift) |
| `vendor/` | Audited forks: `adblock`, `tao`, `tauri`, `tauri-runtime-wry`, `wry`, `tauri-plugin-updater` |
| `assets/` | Blocker seed lists plus licenses, brand logo, kept-site icons, sky images, sounds |
| `eval/` | Work evaluation suites and agentic-browsing evidence manifests |
| `docs/` | Architecture, frontend, Work, security and QA guides; translated READMEs |
| `patches/` | pnpm patch for `braces@3.0.3` |
| `.github/` | CI workflows, release runbook (`RELEASE.md`), templates, dependabot |
| `.cargo/`, `.config/` | `xtask` alias, MSVC control-flow-guard, macOS runner; nextest CI profile |
| `Cargo.toml` | Workspace: `crates/*`, `desktop`, `xtask` (vendor forks are excluded and have their own lockfiles) |
| `package.json`, `pnpm-workspace.yaml` | pnpm workspace (`frame`, `desktop`), security overrides, `dev` / `build` / `build:custom` scripts |
| `deny.toml` | cargo-deny policy: advisories, license allowlist, allowed sources |
| `AGENTS.md`, `CLAUDE.md`, `TODO.md` | Agent rules (upstream and fork) and the shared task list |

## 3. Architecture in one picture

```
frame/ (Svelte)                    desktop/ (Tauri composition root)              crates/
───────────────                    ─────────────────────────────────              ───────
app → features → session           lib.rs: specta_builder() registers             zephium-app  (actor; owns state)
        → domain → shared/ipc ───►   ~300 #[tauri::command]s, plugins,     ───►      └► zephium-core (domain, reducers, ports)
                                     setup(); work_*, webext, updates,                └► zephium-store (SQLite actor)
  ◄── events "zephium:*" ────────    platform/, overlay, panel …                      └► zephium-engine (WebView2/WebKit)
                                                                                      └► agent / Work / blocker crates
```

- **Rust holds all authority.** The frontend only *projects* state and
  *submits typed intents*. There is no second source of truth in the UI.
- `zephium-app` is an **actor**. One thread owns application state. UI
  intents and engine callbacks enter its queue, reducers in `zephium-core`
  change the state, side effects go out through **port traits**
  (`core/src/ports`), and the actor publishes projections.
- IPC is typed with **tauri-specta**. Running the test
  `export_typescript_bindings` (in `desktop/src/lib.rs`) generates
  `frame/src/shared/ipc/bindings.ts`, `work-constants.ts` and
  `theme-presets.json`. Never hand-edit these files.
- A logical tab is application state. It is not the same thing as a live
  native WebView or an OS process.

## 4. Rust crates (`crates/`)

### Dependency layers

- **Leaves** (no zephium dependencies): `core`, `credentials`, `decision`,
  `private-fs`, `update-transport`, `webext`, `voice`
- **On top of core**: `ipc`, `import`, `notes`, `blocker`. `mcp` builds on
  `credentials`, and `page-assist` builds on `ipc`.
- **Agent stack**: `agentic` (on core and decision) →
  `agent-runtime`, `agent-model-catalog`, `agent-provider-transport` →
  `agent-controller`
- **Composition**: `engine` (core, blocker, agentic, webext) · `store`
  (core, agentic, private-fs) · `app` (core, ipc, store, agent crates, mcp) ·
  `work-composition` (app, engine, store, page-assist, agents) · `desktop`
  (everything)

### Crate map

| Crate | Owns | Lines |
| --- | --- | --- |
| **Browser core** | | |
| `zephium-core` | Domain types, reducers, navigation policy, layout math, **port traits**. `forbid(unsafe_code)`. Subfolders: `ports/`, `work/`, `extensions/`, `userscripts/`, `permissions/`, `blocker/`, `search/` | 54k |
| `zephium-app` | Actor shell. `actor/` mailbox; `shell/` (113 files) handles browser orchestration (tabs, history, bookmarks, blocker, extensions, persistence, projections); about 60 `work_*` files plus `work_lead/`, `work_commands/`, `work_computer/`, `work_connections/`. `skills/**/SKILL.md` are **compiled runtime prompts** | 172k |
| `zephium-engine` | Native browser views. `host/` holds platform-neutral host services (navigation, downloads, permissions, scriptlets, webext, previews); `platform/windows` (WebView2/Win32), `platform/macos` (WKWebView/AppKit), `platform/linux` (unsupported); 9 probe binaries in `bin/` | 170k |
| `zephium-ipc` | Typed Rust→frame DTOs. Pure data, no Tauri | 4k |
| **Storage and files** | | |
| `zephium-store` | Crash-aware SQLite behind one actor. `hub/` holds per-domain tables; migrations and SQL schemas | 45k |
| `zephium-private-fs` | Fail-closed filesystem primitives for private data (leases, namespaces, Windows ACLs) | 15k |
| `zephium-notes` | Markdown notes stored as user-owned files | 4k |
| `zephium-import` | Read-only import from Chromium, Firefox, Safari, Arc, Zen | 4k |
| **Content protection** | | |
| `zephium-blocker` | Deterministic compiler for adblock lists: rules, cosmetics, scriptlets, WebKit content-blocker output | 18k |
| `zephium-blocker-update` | Filter-package validation and authenticated download (TUF) | 13k |
| `zephium-blocker-service` | Production coordination of filter packages and compilation; release seed | 5k |
| `zephium-update-transport` | Fixed-origin HTTPS transport (Chrome store, official filters) | 1k |
| **Extensions** | | |
| `zephium-webext` | Platform-neutral CRX3 verification, extraction, manifest, permissions; Windows JS shims | 4k |
| `zephium-webext-macos` | WebKit `WKWebExtension` runtime, one module per `chrome.*` API | 11k |
| `zephium-webext-windows` | Windows extension lab and qualification only (not integrated) | 1k |
| **Agents and Work** | | |
| `zephium-agentic` | Domain contracts for agentic browsing: policy, semantic actions (`semantic_*`), `work_browser_*`, providers (`agent_provider/`: Anthropic, OpenAI, SSE), `provider_transport/` (opt-in network) | 190k |
| `zephium-agent-controller` | Bounded browser controllers (Terra; work navigation, forms, listing, discovery) | 34k |
| `zephium-agent-runtime` | Execution ownership: mailbox, scoped runtime | 6k |
| `zephium-agent-model-catalog` | Immutable model and pricing catalog | 1k |
| `zephium-agent-provider-transport` | Façade over the optional provider transport | <1k |
| `zephium-decision` | Typed probabilistic answers over code-enumerated options | 2k |
| `zephium-work-composition` | Trusted desktop Work composition (no Tauri/UI); `page_assist/` sidebar service; qualification objectives | 31k |
| `zephium-page-assist` | Pure core of Ask Zephium: page framing, citations, pacing, conversation, policy | 5k |
| `zephium-mcp` | MCP client over `rmcp` (stdio; HTTP with OAuth) | 3k |
| **System** | | |
| `zephium-credentials` | OS vault (Windows Credential Manager, macOS Keychain) | <1k |
| `zephium-voice` | Dictation: mic capture (WASAPI, Apple Speech), level meter | 2k |
| **Qualification** | | |
| `zephium-terra-macos-probe` | Release-excluded live agent probes (macOS and Windows binaries) | 23k |

Probe and qualification code (`*_probe*`, `*_qualification*`,
`terra-macos-probe`, `webext-windows`) **is not shipping behaviour**. Do not
use it as evidence that a feature works.

## 5. Desktop app (`desktop/`)

- `src/lib.rs` (about 10.5k lines) is the composition root.
  `specta_builder()` (line ~1743) collects about **300 commands** and the
  events. `run()` (line ~6847) registers plugins (dialog, updater,
  single-instance, global-shortcut), the invoke handler and `setup()`.
- `src/` modules (120 files):

| Group | Modules |
| --- | --- |
| Work | `work.rs`, `work_*` (activity, alerts, connections, decision, folders, models, product, provider, …), behind the `work-product` feature |
| Updates | `updates.rs`, `updates/{artifact,recovery}.rs` |
| Extensions | `webext/`, `webext_qa.rs` |
| Platform chrome | `platform/` (`windows*.rs` for caption, menus, renderer, pointer; `macos/`), `default_browser/` |
| Windows/overlays | `overlay/`, `panel.rs`, `tab_switcher`, `tab_previews`, `history_menu` |
| Browser features | `blocker_service`, `browser_import`, `profiles`, `private_window`, `spaces`, `notes`, `page_assist`, `search_providers`, `media`, `voice`, … |
| System | `login_item`, `taskbar_badge`, `memory_pressure`, `relaunch`, `focus_alerts`, `windows_privacy`, `frame_sources`, … |
| Diagnostics | `diagnostic_report/` (redaction), `legacy_vault.rs` (fork: one-time removal of the old Zephium account token) |
| Probes | `navigation_probe`, `foreground_rendering_probe`, `favicon_probe`, `blocker_bench` |

- **Tauri configs** (each one is merged over `tauri.conf.json`):

| File | Identity / purpose |
| --- | --- |
| `tauri.conf.json` | Official `Zephium` / `app.zephium`; updater uses the owner's releases |
| `tauri.windows.conf.json` / `tauri.macos.conf.json` | Platform overlays (frameless window, NSIS per-user install / DMG, entitlements) |
| `tauri.dev.conf.json` | `Zephium Dev` / `app.zephium.dev`, used by `pnpm dev` |
| `tauri.custom.conf.json` | **Fork:** `Zephium Custom` / `app.zephium.custom`, NSIS only, updater pointed at this fork; used by `pnpm build:custom` |
| `tauri.*-qa*.conf.json`, `tauri.performance.windows.conf.json`, `tauri.work-*.conf.json` | Isolated QA, probe and performance identities |

- Other: `capabilities/` (Tauri ACL), `windows/installer-hooks.nsh` (NSIS
  uninstall cleanup), `installer/` art, `tests/` (probe-config tests),
  `windows-log-tests/` (separate crate).

## 6. Frontend (`frame/`)

**Stack:** Svelte 5 (runes), Vite 8, Tailwind 4, TypeScript 6, bits-ui,
TipTap (notes), xyflow (graphs). Tests use Vitest in two projects: `unit`
(Node) and `component` (browser mode; Chromium on Windows, WebKit on macOS).
Linting uses ESLint plus `eslint-plugin-boundaries`, Stylelint, Prettier and
Knip. i18n uses inlang/Paraglide with 13 locales in `messages/`; the output in
`src/shared/i18n/` is generated.

### Entry points

| Page | Bootstrap | Root component |
| --- | --- | --- |
| `browser.html` | `src/entries/browser.ts` | `src/app/browser/BrowserApp.svelte` (window `main`) |
| `panel.html` | `src/entries/panel.ts` | `src/app/panel/PanelApp.svelte` (window `panel`, launcher) |
| `onboarding.html` | `src/entries/onboarding.ts` | `src/app/onboarding/OnboardingApp.svelte` (separate build) |

### Layers in `frame/src` (enforced by `architecture.config.js`)

```
entries → app → features → session → domain → shared      (all may import styles)
```

| Layer | Role |
| --- | --- |
| `app/` | Composition roots (browser, panel, onboarding) |
| `features/` | 28 feature modules, each with an `index.ts` public API plus `components/`, `lib/`, `tests/`. **No sibling-feature imports** except listed exceptions (`tools`→notes/tasks/history/…, `settings`→time/assist, `assist`→work, `library`→downloads, `work`→tasks/notes/library) |
| `session/` | Transient per-document state (drag, attention, tool drafts, sidebar mode, …) |
| `domain/` | 37 slices that mirror Rust projections and send typed intents. **Only `domain` talks to IPC** |
| `shared/` | `ipc/` (generated `bindings.ts` plus hand-written `native-events.ts`), `ui/` primitives, `lib/` helpers, `i18n/` (generated), `sky/`, `testing/` (test-only) |
| `styles/` | `global.css`, `tokens.css`, `tokens/*`, `axes/*` (palette, tint, density, contrast, material, motion, type) |

Feature modules: `address`, `assist`, `blocker`, `bookmarks`,
`dock`, `downloads`, `essentials`, `extensions`, `history`, `library`,
`media`, `newtab`, `notes`, `onboarding`, `panel`, `permissions`, `search`,
`settings`, `sidebar`, `source`, `split`, `tabs`, `tasks`, `time`, `tools`,
`updates`, `webext`, `work` (the largest, about 419 files).

Path aliases: `$app`, `$features`, `$session`, `$domain`, `$shared`, `$styles`.
`@tauri-apps/api/core` may be imported only inside `src/shared/ipc/**`, and
`@tauri-apps/api/event` is banned everywhere.

**IPC pattern:** a domain slice imports `commands` from
`$shared/ipc/bindings` and `events` from `$shared/ipc/native-events`,
subscribes with `events.x.listen(...)` and calls `await commands.y(...)`.
To add data: change the Rust projection, regenerate the bindings, then add
the domain mirror.

## 7. Tooling, CI and release

- **`cargo xtask`**: `ci` (local aggregate), `check-*-boundary`,
  `check-engine-floors`, `check-webview2-floor`,
  `check-advisory-exceptions`, `check-security-fork-locks`,
  `update-blocker-seed` / `check-blocker-seed`, `blocker-bench`,
  `webext-suite`, `sign-windows-updater`, `check-release-assets`.
- **`scripts/`**: `require-supported-platform.mjs`; `ci/` (dependency audit,
  action pins, path scoping, flaky summary); `release/` (tag, SignPath,
  third-party notices); `qualification/` (Windows QA PowerShell); blocker
  bench, corpus and scriptlets; standalone JS tests.
- **Workflows**: `ci.yml` (frontend, policy, macOS, Windows), `codeql.yml`,
  `components.yml` (WebKit component shards), `nightly.yml`,
  `security-review.yml`, `release.yml` (tag `v*`: build, sign, draft),
  `publish-stable.yml`.
- **Vendored forks** (`vendor/`, each with `UPSTREAM.md` / `FORK.toml` /
  `REBASE.md`):

| Fork | Base | Why |
| --- | --- | --- |
| `adblock` | brave/adblock-rust 0.13.2 | network-policy adapter |
| `tao` | 0.35.3 | Windows fullscreen fix, mouse capture, session-end hooks |
| `tauri` | 2.11.3 | ephemeral Linux incognito WebViews |
| `tauri-runtime-wry` | 2.11.3 | exact async window/WebView creation |
| `wry` | 0.55.1 | security hardening (deny-by-default file picker, …) |
| `tauri-plugin-updater` | 2.10.1 | atomic swap, bounded metadata, signature before install |

## 8. Common commands (repository root, Windows)

| Goal | Command |
| --- | --- |
| Install JS deps | `pnpm install --frozen-lockfile` |
| Run dev app (separate identity) | `pnpm dev` |
| Build fork installer | `pnpm build:custom` → `target\release\bundle\nsis\Zephium Custom_<ver>_x64-setup.exe` |
| Frontend gate | `pnpm --dir frame run check`, `pnpm --dir frame run build`, `pnpm --dir frame run check:styles` |
| Rust format / lint | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Rust tests (Windows CI set) | `cargo nextest run --workspace --exclude zephium-desktop --locked --profile ci` (see CONTRIBUTING for exclusions) |
| Regenerate IPC bindings | `cargo test -p zephium-desktop --locked export_typescript_bindings` |
| Regenerate i18n | `pnpm --dir frame run i18n` |
| Sync with the owner | `git fetch upstream`, then `git merge upstream/main` |

## 9. Where to make common changes

| Change | Start in |
| --- | --- |
| UI of an existing feature | `frame/src/features/<feature>/components` |
| New data shown in the UI | Rust projection (`zephium-core` / `zephium-app`), then `zephium-ipc` DTO, then regenerate bindings, then `frame/src/domain/<slice>` |
| New native command | `desktop/src/<module>.rs` plus registration in `specta_builder()` |
| Browser behaviour (tabs, navigation) | `zephium-core` reducers, then `zephium-app/src/shell` |
| WebView2-specific behaviour | `zephium-engine/src/platform/windows` |
| Persistence | `zephium-store/src/hub/<domain>` plus a migration |
| Ad blocking | `zephium-blocker*` (read `docs/adblock.md` first) |
| Work / agents | `zephium-app/src/work_*`, `zephium-agentic`, `zephium-work-composition`, `frame/src/features/work` (read `docs/work.md`) |
| Theme / colours | `frame/src/styles/tokens.css` and `styles/axes/*` |
| Translations | `frame/messages/<locale>.json` |
| App name, ID, installer, updater of the fork | `desktop/tauri.custom.conf.json` |

## 10. Known state

- The source has almost no `TODO`/`FIXME` markers. Debt is tracked in
  `frame/knip-migration.ts`, `frame/stylelint-migration.js`,
  `frame/bundle-budgets.json` and the advisory exceptions in `deny.toml`.
- `frame/architecture.config.js` lists feature exceptions for `focus`,
  `activity` and `resources`, which do not exist as folders. They are stale
  or forward-looking.
- Linux code exists (`engine/platform/linux`, `tauri.linux.conf.json`) but
  is unsupported.
