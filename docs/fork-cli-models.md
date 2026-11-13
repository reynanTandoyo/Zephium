# Fork design: AI apps on this PC as model backends (T-0008)

Status: **proposed**, not implemented. Owner: fork `reynanTandoyo/Zephium`.

## Goal

Work and Ask Zephium use the person's **subscription** through AI command-line
apps installed on this PC, with no API key:

| App | Command | Installed here (2026-11-13) |
| --- | --- | --- |
| Claude Code | `claude` | yes, 2.1.319 |
| Antigravity CLI | `agy` | yes, 1.5.8 |
| OpenCode | `opencode` | no |

The custom endpoint (OpenAI-compatible, base URL + model + optional key) stays as
the backup. The built-in key providers (Anthropic, OpenAI, Gemini, …) are hidden
from the interface, but their code stays.

Settings → AI gets an **"AI apps on this PC"** section that works like plugins.
Zephium scans for each app and shows its state: *Installed · signed in*,
*Installed · sign in needed*, or *Not installed*. A missing app gets an
**Install** button.

## How it plugs in (seam)

Every model call goes through one trait, `WorkModelClient`
(`zephium-core/src/work/model.rs`), which `client()` in
`zephium-app/src/work_models.rs` creates. A CLI backend is a new
`WorkModelClient`, so Work, its helpers, the browser page agent, decisions and
Ask Zephium all use it without changes.

- New `WorkModelProvider` variants `ClaudeCli`, `AntigravityCli`, `OpenCodeCli`
  and a new `WorkModelWire::CliJson`.
- A new fork-owned crate `crates/zephium-cli-models` (keeps upstream merge
  conflicts small) with:
  - `locate` / `status`: the path, version and sign-in state of each app.
  - `CliModelClient`: one call = one CLI run in print mode with JSON output.
- Tool calling: Zephium's request carries native tools. The client sends the
  tool list in the prompt and forces the answer through `--json-schema`:
  `{ "text": string, "calls": [ { "name": <enum of tool names>, "arguments": object } ] }`.
  The client then turns `calls` into `WorkModelPart::ToolCall` parts and
  `text` into a `Text` event (Ask Zephium shows only `Text` events). Tested on
  2026-11-13: Claude Code returned a valid structured step in 3.4 s.

### Claude Code command (per call; the prompt goes on stdin, never in argv)

```
claude -p --output-format json --no-session-persistence --model <alias>
  --system-prompt <Zephium system text> --setting-sources "" --strict-mcp-config
  --disallowedTools <every built-in tool> --json-schema <schema>
```

- Do not use `--bare`, because it ignores the subscription login.
- `total_cost_usd` and `usage` in the result map to `WorkModelUsage`.
  Subscription use is shown as "Subscription", not as a charge.

### Antigravity (`agy`) command

```
agy -p --output-format json --model <id> --effort <level> --json-schema <schema>
  --disable-slash-commands
```

Models come from `agy models`. Before shipping, find a way to switch off
`agy`'s built-in tools, such as `--settings` or an agent without tools. If
none exists, `agy` stays text-only and is not offered for the browser page
agent.

### OpenCode

Not installed yet. Its non-interactive and server modes (`opencode run`,
`opencode serve`) must be checked once the Install button installs it.

## Safety rules (from docs/security-model.md and docs/work.md)

- The CLI is a **model**, not an agent. Its own tools (shell, file edits, web,
  MCP) are switched off for every call, and its sessions are not saved.
  Otherwise it would become the "coding-agent hand-off" that upstream keeps
  disabled.
- Spawn the CLI like `zephium-mcp` spawns stdio servers: an absolute program
  path, an explicit environment (adding `USERPROFILE`, `APPDATA`,
  `LOCALAPPDATA`, `SystemRoot`, `PATHEXT` so the CLI finds its login), no
  console window, a kill-on-close job object, bounded output (8 MiB), a
  timeout per call, and dropping the call kills the process.
- CLI output is untrusted input. `WorkModelError` stays closed, with no CLI
  text in errors or logs.
- Images (screenshots): version 1 marks CLI models `vision: false`.
- Cost bookkeeping: CLI entries count as free (like local endpoints), so
  Work does not reserve paid rates for them.

## Install button

The button shows the app's official install command and, after the person
confirms, runs it in a visible terminal window:

- Claude Code: the official Windows installer from Anthropic's docs.
- OpenCode and Antigravity: their official install methods.

The exact commands are checked against each vendor's current documentation
during implementation. If an app has no scriptable installer, the button
opens its official download page instead. After installing, the person signs
in once in that terminal (`claude` / `agy` / `opencode`), then clicks
**Check again**.

## IPC contract (shared by the Rust and frame work)

Rust (`zephium-core` `work/model.rs`):

- `WorkModelProvider` gains `ClaudeCode` (serde `"claude_code"`), `Antigravity`
  (`"antigravity"`) and `OpenCode` (`"open_code"`). Entry id slugs are
  `claude-code/<model>`, `antigravity/<model>` and `opencode/<model>`.
- `WorkModelWire` gains `Cli` (`"cli"`).
- `WorkModelSupports` for CLI entries: `tools: true`, `vision: false`,
  `prompt_cache: false`, `reasoning: true` (Claude, agy effort),
  `native_search: false`. Their price is zero.

Desktop (fork-owned `desktop/src/ai_apps.rs`, registered in `specta_builder()`):

```rust
#[serde(rename_all = "snake_case")] enum WorkAiAppIdV1 { ClaudeCode, Antigravity, OpenCode }
#[serde(rename_all = "snake_case")] enum WorkAiAppStateV1 { Missing, SignInNeeded, Ready, Unknown }
struct WorkAiAppInstallV1 { command: Option<String>, url: String }
struct WorkAiAppV1 {
    app: WorkAiAppIdV1, provider: WorkModelProvider, name: String,
    state: WorkAiAppStateV1, version: Option<String>, install: WorkAiAppInstallV1,
}
struct WorkAiAppsV1 { apps: Vec<WorkAiAppV1> }
```

Commands (TypeScript names after regeneration):

- `commands.workAiApps(rescan: boolean): Promise<WorkAiAppsV1>`: scans for
  the apps. `rescan: false` returns the cached scan.
- `commands.workAiAppInstall(app: WorkAiAppIdV1): Promise<Result<null, WorkModelsFaultV1>>`:
  opens a visible terminal that runs the official install command, or opens
  `install.url` in a tab when `command` is null.
- `commands.workAiAppSignIn(app: WorkAiAppIdV1): Promise<Result<null, WorkModelsFaultV1>>`:
  opens a visible terminal running the app's sign-in.
- After a rescan that changes anything, Rust emits the existing
  `zephium:work-models-changed` event.
- `WorkProviderStatusV1` gains `app: Option<WorkAiAppV1>`, which is set for the
  three app providers. An app provider is usable only when
  `app.state == "ready"`.

## Phases

| Phase | Work | Files |
| --- | --- | --- |
| 1 | Crate `zephium-cli-models`: locate, status, spawn helper, `CliModelClient` for Claude Code, unit tests with a fake CLI | new crate; `zephium-core` model.rs variants |
| 2 | Register providers: `client()`, `usable()`, catalog entries (Claude: `fable`, `opus`, `sonnet`, `haiku`; agy from `agy models`), free accounting, IPC status DTO | `zephium-app` work_models.rs, agentic `lead/keys.rs` + `models.rs` matches, desktop `work_models.rs`, regenerate bindings |
| 3 | Antigravity client (same client with different arguments and parser) | crate |
| 4 | Interface: "AI apps on this PC" rows with Install / Check again, picker seats, onboarding option, hide built-in key providers, translations | `frame/src/features/settings`, `domain/ai`, `features/work/bar`, `features/onboarding`, `frame/messages` |
| 5 | OpenCode client after installing it | crate |
| 6 | Checks: clippy, Rust tests, frame check, `pnpm build:custom`, a real Work run and an Ask Zephium question through Claude Code | — |
