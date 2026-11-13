# TODO

Shared task list for this fork (`reynanTandoyo/Zephium`). People and agents
both read and edit it. Several agents may run at the same time, so **every
edit to this file goes through the lock below**. Nobody needs the lock to read
the file.

## Editing protocol (mandatory)

The lock is the directory `.todo.lock` in the repository root. Creating a
directory is atomic: when two agents try at once, only one succeeds.

1. **Acquire**: create the lock directory and write your owner note. If
   `mkdir` fails, another agent holds the lock. Wait 5–15 s and try again.
   Never edit this file without the lock.

   ```bash
   mkdir .todo.lock && echo "<agent-name> $(date -u +%Y-%m-%dT%H:%M:%SZ)" > .todo.lock/owner
   ```

   ```powershell
   cmd /c mkdir .todo.lock; if ($LASTEXITCODE -eq 0) { "<agent-name> $((Get-Date).ToUniversalTime().ToString('s'))Z" | Out-File -Encoding utf8 .todo.lock\owner }
   ```

2. **Re-read** `TODO.md` *after* you have the lock. Never edit from a copy you
   read before acquiring it.
3. **Edit** briefly: add, claim or update your entries. Touch only the lines
   you mean to change, and never rewrite or reorder other agents' entries.
   Do not run builds or tests while you hold the lock.
4. **Release** right after saving, in every case, including when the edit
   fails:

   ```bash
   rm -rf .todo.lock
   ```

   ```powershell
   Remove-Item -Recurse -Force .todo.lock
   ```

5. **Stale lock**: a lock whose `owner` timestamp is more than **10 min** old
   has been abandoned. Delete it and acquire it again. Never delete a newer
   lock held by another agent.

`.todo.lock/` is ignored by git and must never be committed.

## Entry format

One task is one bullet in exactly one status section:

```
- [ ] T-0007 · <short imperative title> · area: <folder or crate> · owner: <agent/person or -> · updated: YYYY-MM-DD
  - <optional detail lines: context, acceptance criteria, links to files>
```

- **IDs**: take the next number after `Last ID`, then increase `Last ID`. Both
  happen in the same locked edit. IDs are never reused.
- **Claim**: move the entry to *In progress* and set `owner` to yourself.
  Do not take a task someone else owns.
- **Finish**: change `[ ]` to `[x]`, move the entry to *Done* and add one
  detail line saying what changed and how it was verified. Commit hashes or
  branches go here too.
- **Blocked**: move the entry to *Blocked* and add a detail line naming what
  it waits for.
- Keep entries short. Design notes belong in `docs/` or a PR, not here.

Last ID: T-0009

## In progress

- [ ] T-0007 · Remove the Zephium account and the hosted "Zephium AI" (Cloud) provider · area: zephium-account, desktop/src/account*, Cloud provider code, frame account · owner: claude-main (+ subagents) · updated: 2026-11-13
  - Branch `remove-account` (from `custom`). The account holds only identity, plan, usage, billing and avatar on `api.zephium.app`. Browser settings are local.
  - **Keep** `zephium-app` `work_account*` and `WorkOperationV1::PrepareAccount`: these handle *website* account approvals, not the Zephium account.
  - Keep the persisted serde variants (`WorkModelProvider::Cloud`, hosted `WorkModelFaultV1`, `PageAssistCostV1::Hosted`) as deprecated and never produced, so saved runs and conversations still load.
  - 2026-11-13: committed on `remove-account`. Passing: `cargo clippy --workspace --all-targets --locked -- -D warnings`, tests for app/agentic/work-composition/page-assist, `pnpm --dir frame run check` (1336 tests), frame build and `check:styles`. 10 `zephium-app` Windows tests (work_files, work_personal, work_lead, export) fail identically on the baseline. Component tests were not run (no browser available).
  - 2026-11-13: the release build launches and shows its window (see T-0009). Remaining: merge `remove-account` into `custom` once the user approves.

## Open

- [ ] T-0002 · Launch `pnpm dev` and confirm "Zephium Dev" starts · area: desktop, frame · owner: - · updated: 2026-11-13
  - Must not touch the installed Zephium or its profile.
- [ ] T-0004 · Decide on fork releases for colleague updates · area: .github, desktop · owner: - · updated: 2026-11-13
  - Today the custom updater endpoint points at this fork's releases, and none exist, so no automatic updates happen.
  - Automatic updates would need a minisign key pair, a `pubkey` in `tauri.custom.conf.json` and published `latest.json` and installers.
- [ ] T-0006 · Remove macOS-specific code (Windows-only fork) · area: crates, desktop, frame, xtask, scripts, .github · owner: - · updated: 2026-11-13
  - Scope today: 158 macOS-named `.rs` files (about 83k lines), `zephium-webext-macos` (11k) and `zephium-terra-macos-probe` (23k), 1,895 `target_os = "macos"` sites in 188 files, 29 frame files, Swift, plist and macOS CI jobs.
  - Decide the scope first: whole files and crates only, or also the `cfg` blocks. The `cfg` edits in shared files cause conflicts on every upstream merge.
  - Acceptance: Windows `cargo clippy --workspace --all-targets --locked -- -D warnings`, the Windows nextest set and `pnpm --dir frame run check` pass, and `pnpm build:custom` works.
- [ ] T-0008 · Replace built-in AI providers with Claude CLI, Antigravity CLI, OpenCode CLI and a custom BYOK backup · area: zephium-agentic, zephium-app work_*, desktop work_*, frame work/assist/settings · owner: - · updated: 2026-11-13
  - Needs a design decision first: (a) a CLI as the model behind Work's own tool loop, (b) handing whole tasks to the CLI (`work_computer/delegate.rs` already knows Codex and Claude, but it is gated off), or (c) Zephium exposing the browser as an MCP server that the CLI drives. Also decide what happens to the Ask Zephium sidebar and voice dictation.
  - Keep the existing OpenAI-compatible "Compatible" BYOK provider (endpoint, model, key) as the backup. Remove the curated providers (Anthropic, OpenAI, Gemini, xAI, Mistral, DeepSeek, Groq, Cerebras, OpenRouter) and Zephium AI.
  - Check whether Antigravity has a scriptable CLI before committing to it.
- [ ] T-0005 · Remove stale feature exceptions `focus`/`activity`/`resources` · area: frame/architecture.config.js · owner: - · updated: 2026-11-13
  - These folders do not exist in `frame/src/features`. Low priority; consider reporting it upstream instead.

## Blocked

## Done

- [x] T-0009 · "Zephium Custom" refused to start: "private filesystem path or node is unsafe" · area: zephium-private-fs, zephium-store · owner: claude-main · updated: 2026-11-13
  - Cause: Windows Work storage accepts only a closed list of app identities, and `app.zephium.custom` was not on it. Added `NativeApplication::Custom` (folder `app.zephium.custom`), its mapping in `WindowsWorkStorage::for_application` and its test entry. An early theory about the `S-1-15-3-…` ACE on `%USERPROFILE%\AppData` was wrong, and that ACE needs no change.
  - Verified: `cargo test -p zephium-store --features work-execution application_selection`, workspace clippy, `pnpm build:custom`, then a release launch showed the window after 784 ms.
- [x] T-0001 · Build and verify the "Zephium Custom" installer · area: desktop · owner: claude-main · updated: 2026-11-13
  - `Zephium Custom_1.0.5_x64-setup.exe` built and the release executable starts with its own profile `%APPDATA%\app.zephium.custom`. Not yet installed through the installer.

- [x] T-0003 · Commit the fork files and push branch `custom` to `origin` · area: repo root · owner: claude-main · updated: 2026-11-13
  - Commit `0564fa74a` on `custom`, pushed to `origin/custom` and tracking it. `main` stays a clean mirror of upstream.
