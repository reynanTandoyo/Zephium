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

Last ID: T-0046

## In progress

## Open

- [ ] T-0010 · Modes: define the mode list as data · area: frame/src/domain/surface, frame/src/features/sidebar · owner: - · updated: 2026-11-13
  - `{ page, label, icon, feature }` in order Browse, Work, Mail, Calendar; a mode is hidden when its feature is off. Today "Browse" means no full-window page and "Work" is `BrowserPage::Work`.
- [ ] T-0011 · Modes: `ModeTabs` shows N modes · area: frame/src/features/sidebar/components/ModeTabs.svelte · owner: - · updated: 2026-11-13
  - Replace the two-option segmented control with one driven by T-0010; update `ModeTabs.component.test.ts`.
- [ ] T-0012 · Modes: `ModeWord` rail switch shows N modes · area: frame/src/features/sidebar/components/ModeWord.svelte · owner: - · updated: 2026-11-13
  - N-cell grid and moving thumb instead of `repeat(2, …)` and `data-work=true/false`.
- [ ] T-0013 · Modes: generalise "in Work" into "in a full-window mode" · area: frame/src/app/browser/Shell.svelte, Sidebar.svelte, session/tools.svelte.ts, AddressField, SidebarSearch, BrowserApp · owner: - · updated: 2026-11-13
  - About 60 frontend lines check `inWork` / `=== "work"`. Shared mode behaviour: compact rail, no address field, tabs open over the mode, follows space changes. Keep Work-only parts (canvas, Work pane, tool redirect) for Work. Update `Shell` and `surface` tests.
- [ ] T-0014 · Modes: Rust `BrowserPage::Mail` and `BrowserPage::Calendar` · area: crates/zephium-app/src/api.rs, desktop/src/lib.rs (`browser.*` match), crates/zephium-core/src/commands.rs · owner: - · updated: 2026-11-13
  - Variants, `command_id`, `feature()`, `feature_of`, the `browser.*` match, and regenerated bindings.
- [ ] T-0015 · Modes: make Work-only Rust branches mode-aware · area: crates/zephium-app/src/shell/{spaces,operations,time,media,browser_pages}.rs · owner: - · updated: 2026-11-13
  - Switching space keeps the mode, typing an address returns to Browse, time tracking records the mode. Update the Rust tests in `shell/tests`.
- [ ] T-0016 · Modes: generated native `mode.choose` menu · area: desktop/src/lib.rs (~4024-4044) · owner: - · updated: 2026-11-13
  - Today it is hard-coded to Browse and Work.
- [ ] T-0017 · Modes: Mail and Calendar switches in Settings → Features · area: frame/src/features/settings/lib/catalog.ts, frame/src/domain/preferences · owner: - · updated: 2026-11-13
  - `mail.enabled`, `calendar.enabled`; turning one off while in its mode returns to Browse (like `operations.rs` does for Work).
- [ ] T-0018 · Modes: texts and the onboarding mock switch · area: frame/messages (13 locales), frame/src/features/onboarding/components/Window.svelte · owner: - · updated: 2026-11-13
  - `mode_mail`, `mode_calendar`, update `ui_mode_work_hint`; the onboarding window draws a fake two-tab switch.
- [ ] T-0019 · Mail: check the company server type in Thunderbird · area: (user) · owner: user · updated: 2026-11-13
  - Thunderbird → Account Settings → Server Settings: IMAP / POP / Exchange, and the server names; also whether the work calendar is CalDAV or Exchange. Decides T-0034 and T-0040.
- [ ] T-0020 · Mail: crate `zephium-mail` with IMAP over TLS · area: new crates/zephium-mail · owner: - · updated: 2026-11-13
  - `async-imap` + rustls; IDLE for push; bounded reads; all server data is untrusted input. Unit tests against a fake IMAP server.
- [ ] T-0021 · Mail: MIME parsing · area: crates/zephium-mail · owner: - · updated: 2026-11-13
  - `mail-parser`: headers, text, HTML, inline images, attachments, encodings; size limits.
- [ ] T-0022 · Mail: account setup with automatic server lookup · area: crates/zephium-mail, desktop IPC `mail_*`, frame/src/features/mail · owner: - · updated: 2026-11-13
  - Address + password; Mozilla ISPDB / `autoconfig.<domain>` / `.well-known/autoconfig` / SRV lookup; manual host, port and security fallback; "Test connection".
- [ ] T-0023 · Mail: passwords in Windows Credential Manager · area: crates/zephium-credentials, crates/zephium-mail · owner: - · updated: 2026-11-13
  - Never in the store, logs, diagnostics or IPC.
- [ ] T-0024 · Mail: several accounts per profile · area: zephium-store, frame/src/features/mail · owner: - · updated: 2026-11-13
  - Display name, colour, signature; reorder and remove accounts.
- [ ] T-0025 · Mail: local cache in the store · area: crates/zephium-store (new mail tables + migration) · owner: - · updated: 2026-11-13
  - Headers, flags, bodies on demand; bounded size; sync state per folder (UIDVALIDITY, MODSEQ/CONDSTORE).
- [ ] T-0026 · Mail: folder list and combined inbox · area: frame/src/features/mail, crates/zephium-mail · owner: - · updated: 2026-11-13
  - Inbox, Sent, Drafts, Trash, Archive, Junk via SPECIAL-USE; unread counts; "All inboxes".
- [ ] T-0027 · Mail: message list with threads · area: frame/src/features/mail · owner: - · updated: 2026-11-13
  - Sender, subject, date, snippet, unread/flag/attachment marks; threading by `References`/`In-Reply-To`; virtual scrolling (`shared/lib/virtual-window`).
- [ ] T-0028 · Mail: safe reading pane · area: frame/src/features/mail, desktop (locked-down view) · owner: - · updated: 2026-11-13
  - Text and HTML with no scripts; remote images blocked until "Load images"; links open as browser tabs and show their real target; attachments save/open. Follow docs/security-model.md.
- [ ] T-0029 · Mail: sending via SMTP · area: crates/zephium-mail · owner: - · updated: 2026-11-13
  - `lettre`, STARTTLS/SSL, auth; store a copy in Sent where the server does not.
- [ ] T-0030 · Mail: compose, reply, reply all, forward · area: frame/src/features/mail · owner: - · updated: 2026-11-13
  - Plain text first, simple rich text later; attachments; signatures; "From" picker across accounts; drafts saved to the Drafts folder.
- [ ] T-0031 · Mail: message actions · area: frame/src/features/mail, crates/zephium-mail · owner: - · updated: 2026-11-13
  - Mark read/unread, flag, move/copy (drag to folder), delete to Trash, archive, mark junk; keyboard shortcuts.
- [ ] T-0032 · Mail: search · area: crates/zephium-mail, zephium-store · owner: - · updated: 2026-11-13
  - Local search of cached mail plus server-side IMAP SEARCH.
- [ ] T-0033 · Mail: new-mail notifications and unread badge · area: desktop/src/focus_alerts.rs, frame/src/features/sidebar · owner: - · updated: 2026-11-13
  - Windows toast; unread count on the Mail mode button.
- [ ] T-0034 · Mail: Microsoft 365 / Gmail sign-in (OAuth2, XOAUTH2) · area: crates/zephium-mail · owner: - · updated: 2026-11-13
  - Needs app registrations with Microsoft and Google (client IDs), done by the user. Only if T-0019 shows it is needed, or for personal accounts.
- [ ] T-0035 · Mail: Exchange support (EWS or Microsoft Graph) · area: crates/zephium-mail · owner: - · updated: 2026-11-13
  - Only if the company server is Exchange without IMAP (see T-0019).
- [ ] T-0036 · Mail: extras · area: crates/zephium-mail, frame/src/features/mail · owner: - · updated: 2026-11-13
  - POP3 (optional), offline mode, rules/filters, contact suggestions from sent mail, `mailto:` links opening Zephium's compose.
- [ ] T-0037 · Calendar: crate and iCalendar parsing · area: new crates/zephium-calendar · owner: - · updated: 2026-11-13
  - `.ics` events with time zones, all-day events and recurrence (RRULE, EXDATE, overrides), e.g. the `icalendar`/`rrule` crates; bounded input.
- [ ] T-0038 · Calendar: CalDAV accounts · area: crates/zephium-calendar, desktop IPC `calendar_*` · owner: - · updated: 2026-11-13
  - Discovery via `.well-known/caldav` and the email domain; username/password in Credential Manager (shared with T-0023).
- [ ] T-0039 · Calendar: ICS subscription links · area: crates/zephium-calendar · owner: - · updated: 2026-11-13
  - Read-only calendars from secret addresses (Google, Outlook, others).
- [ ] T-0040 · Calendar: Google Calendar / Microsoft Graph accounts (OAuth) · area: crates/zephium-calendar · owner: - · updated: 2026-11-13
  - Shares sign-in with T-0034; only if needed (see T-0019).
- [ ] T-0041 · Calendar: local cache and sync · area: crates/zephium-store (calendar tables + migration), crates/zephium-calendar · owner: - · updated: 2026-11-13
  - Sync tokens / ETags; periodic and manual refresh.
- [ ] T-0042 · Calendar: day, week, month and agenda views · area: frame/src/features/calendar, frame/src/domain/calendar · owner: - · updated: 2026-11-13
  - Several calendars overlaid with colours and show/hide toggles; Today button; mini month navigator.
- [ ] T-0043 · Calendar: create, edit, move and delete events · area: frame/src/features/calendar, crates/zephium-calendar · owner: - · updated: 2026-11-13
  - Writable CalDAV calendars; title, time, all-day, location, notes, recurrence, reminders; drag to move.
- [ ] T-0044 · Calendar: reminders as Windows notifications · area: desktop/src/focus_alerts.rs, crates/zephium-calendar · owner: - · updated: 2026-11-13
- [ ] T-0045 · Calendar: meeting invites from Mail · area: crates/zephium-calendar, crates/zephium-mail · owner: - · updated: 2026-11-13
  - Show `.ics` invitations in Mail; answer Accept / Tentative / Decline (iTIP over SMTP); show attendees and their status.
- [ ] T-0046 · Calendar: links with Time, Tasks and meeting links · area: frame/src/features/time, frame/src/features/tasks, frame/src/features/calendar · owner: - · updated: 2026-11-13
  - Today's events in the Time panel / new tab; link events to Tasks; open Teams/Meet/Zoom links as tabs.
- [ ] T-0002 · Launch `pnpm dev` and confirm "Zephium Dev" starts · area: desktop, frame · owner: - · updated: 2026-11-13
  - Must not touch the installed Zephium or its profile.
- [ ] T-0004 · Decide on fork releases for colleague updates · area: .github, desktop · owner: - · updated: 2026-11-13
  - Today the custom updater endpoint points at this fork's releases, and none exist, so no automatic updates happen.
  - Automatic updates would need a minisign key pair, a `pubkey` in `tauri.custom.conf.json` and published `latest.json` and installers.
- [ ] T-0006 · Remove macOS-specific code (Windows-only fork) · area: crates, desktop, frame, xtask, scripts, .github · owner: - · updated: 2026-11-13
  - Scope today: 158 macOS-named `.rs` files (about 83k lines), `zephium-webext-macos` (11k) and `zephium-terra-macos-probe` (23k), 1,895 `target_os = "macos"` sites in 188 files, 29 frame files, Swift, plist and macOS CI jobs.
  - Decide the scope first: whole files and crates only, or also the `cfg` blocks. The `cfg` edits in shared files cause conflicts on every upstream merge.
  - Acceptance: Windows `cargo clippy --workspace --all-targets --locked -- -D warnings`, the Windows nextest set and `pnpm --dir frame run check` pass, and `pnpm build:custom` works.
- [ ] T-0005 · Remove stale feature exceptions `focus`/`activity`/`resources` · area: frame/architecture.config.js · owner: - · updated: 2026-11-13
  - These folders do not exist in `frame/src/features`. Low priority; consider reporting it upstream instead.

## Blocked

## Done

- [x] T-0008 · Replace built-in AI providers with Claude CLI, Antigravity CLI, OpenCode CLI and a custom BYOK backup · area: zephium-agentic, zephium-app work_*, desktop work_*, frame work/assist/settings · owner: claude-main (+ subagents) · updated: 2026-11-13
  - Needs a design decision first: (a) a CLI as the model behind Work's own tool loop, (b) handing whole tasks to the CLI (`work_computer/delegate.rs` already knows Codex and Claude, but it is gated off), or (c) Zephium exposing the browser as an MCP server that the CLI drives. Also decide what happens to the Ask Zephium sidebar and voice dictation.
  - Keep the existing OpenAI-compatible "Compatible" BYOK provider (endpoint, model, key) as the backup. Remove the curated providers (Anthropic, OpenAI, Gemini, xAI, Mistral, DeepSeek, Groq, Cerebras, OpenRouter) and Zephium AI.
  - Check whether Antigravity has a scriptable CLI before committing to it.
  - Design: `docs/fork-cli-models.md`. Branch `cli-models` (from `custom`).
  - 2026-11-13: implemented and committed on `cli-models`. Claude Code and Antigravity are model backends; OpenCode only has detection, Install and Sign in (no client, so it is never usable). The built-in key providers are hidden in the UI, and the custom endpoint stays. Passing: workspace clippy, `cargo test -p zephium-cli-models` (11 unit + 5 fake-app), app/core/agentic/work-composition tests, `pnpm --dir frame run check` (1345 tests), `pnpm build:custom`, the app starts, and a live Claude Code call returned the right tool call in 1.9 s. The user confirmed Ask Zephium works with Claude Code and Antigravity in the app.
  - Known: agy saves a conversation file per call under `%USERPROFILE%\.gemini\antigravity-cli\conversations` (no flag to stop it), and an agy call takes about 60 s. Screenshots are not sent to the apps (text only).
  - Follow-up fixes: agy's agent allows only its `finish` tool (needed for `--json-schema`), wrapped answers are unwrapped, the model card's `fixed` class no longer collides with Tailwind, each app has its own logo (Claude Code uses the Anthropic mark), and the picker card shows "In use". Merged into `custom` and pushed to `origin/custom`.

- [x] T-0007 · Remove the Zephium account and the hosted "Zephium AI" (Cloud) provider · area: zephium-account, desktop/src/account*, Cloud provider code, frame account · owner: claude-main (+ subagents) · updated: 2026-11-13
  - Branch `remove-account` (from `custom`). The account holds only identity, plan, usage, billing and avatar on `api.zephium.app`. Browser settings are local.
  - **Keep** `zephium-app` `work_account*` and `WorkOperationV1::PrepareAccount`: these handle *website* account approvals, not the Zephium account.
  - Keep the persisted serde variants (`WorkModelProvider::Cloud`, hosted `WorkModelFaultV1`, `PageAssistCostV1::Hosted`) as deprecated and never produced, so saved runs and conversations still load.
  - 2026-11-13: committed on `remove-account`. Passing: `cargo clippy --workspace --all-targets --locked -- -D warnings`, tests for app/agentic/work-composition/page-assist, `pnpm --dir frame run check` (1336 tests), frame build and `check:styles`. 10 `zephium-app` Windows tests (work_files, work_personal, work_lead, export) fail identically on the baseline. Component tests were not run (no browser available).
  - 2026-11-13: the release build launches and shows its window (see T-0009). Merged into `custom` (fast-forward to `beae1f006`) and pushed to `origin/custom`.

- [x] T-0009 · "Zephium Custom" refused to start: "private filesystem path or node is unsafe" · area: zephium-private-fs, zephium-store · owner: claude-main · updated: 2026-11-13
  - Cause: Windows Work storage accepts only a closed list of app identities, and `app.zephium.custom` was not on it. Added `NativeApplication::Custom` (folder `app.zephium.custom`), its mapping in `WindowsWorkStorage::for_application` and its test entry. An early theory about the `S-1-15-3-…` ACE on `%USERPROFILE%\AppData` was wrong, and that ACE needs no change.
  - Verified: `cargo test -p zephium-store --features work-execution application_selection`, workspace clippy, `pnpm build:custom`, then a release launch showed the window after 784 ms.
- [x] T-0001 · Build and verify the "Zephium Custom" installer · area: desktop · owner: claude-main · updated: 2026-11-13
  - `Zephium Custom_1.0.5_x64-setup.exe` built and the release executable starts with its own profile `%APPDATA%\app.zephium.custom`. Not yet installed through the installer.

- [x] T-0003 · Commit the fork files and push branch `custom` to `origin` · area: repo root · owner: claude-main · updated: 2026-11-13
  - Commit `0564fa74a` on `custom`, pushed to `origin/custom` and tracking it. `main` stays a clean mirror of upstream.
