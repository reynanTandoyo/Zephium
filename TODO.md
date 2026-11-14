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

- [ ] T-0036 · Mail: extras · area: crates/zephium-mail, frame/src/features/mail · owner: - · updated: 2026-11-13
  - POP3 (optional), offline mode, rules/filters, contact suggestions from sent mail, `mailto:` links opening Zephium's compose.
- [ ] T-0040 · Calendar: Google Calendar / Microsoft Graph accounts (OAuth) · area: crates/zephium-calendar · owner: - · updated: 2026-11-13
  - Shares sign-in with T-0034; only if needed (see T-0019).
  - 2026-11-13: backend done (`2e9d71508`..`0e618bca7` + META v31 `oauth` account kind): Google Calendar API v3 and Microsoft Graph (calendarView/delta) providers, `calendar_add_oauth_account(grant, name)`, guide updated. Tests: calendar 87, oauth 39, store calendar 9 + migrations 61 pass, clippy clean (Windows, fakes only). Open: `oauth_sign_in` `purposes` argument (mail follow-ups worker), the UI fields in Calendar accounts, occurrence edits on Microsoft unsupported.
- [ ] T-0045 · Calendar: meeting invites from Mail · area: crates/zephium-calendar, crates/zephium-mail · owner: - · updated: 2026-11-13
  - Show `.ics` invitations in Mail; answer Accept / Tentative / Decline (iTIP over SMTP); show attendees and their status.
- [ ] T-0002 · Launch `pnpm dev` and confirm "Zephium Dev" starts · area: desktop, frame · owner: - · updated: 2026-11-13
  - Must not touch the installed Zephium or its profile.
- [ ] T-0004 · Decide on fork releases for colleague updates · area: .github, desktop · owner: - · updated: 2026-11-13
  - Today the custom updater endpoint points at this fork's releases, and none exist, so no automatic updates happen.
  - Automatic updates would need a minisign key pair, a `pubkey` in `tauri.custom.conf.json` and published `latest.json` and installers.
  - 2026-11-13: skipped by user for this batch.
- [ ] T-0006 · Remove macOS-specific code (Windows-only fork) · area: crates, desktop, frame, xtask, scripts, .github · owner: - · updated: 2026-11-13
  - Scope today: 158 macOS-named `.rs` files (about 83k lines), `zephium-webext-macos` (11k) and `zephium-terra-macos-probe` (23k), 1,895 `target_os = "macos"` sites in 188 files, 29 frame files, Swift, plist and macOS CI jobs.
  - Decide the scope first: whole files and crates only, or also the `cfg` blocks. The `cfg` edits in shared files cause conflicts on every upstream merge.
  - Acceptance: Windows `cargo clippy --workspace --all-targets --locked -- -D warnings`, the Windows nextest set and `pnpm --dir frame run check` pass, and `pnpm build:custom` works.
- [ ] T-0047 · Fix the AI apps mock in settings component tests · area: frame/src/features/settings/tests, frame/src/domain/ai · owner: - · updated: 2026-11-14
  - `FeaturesLook.component.test.ts` passes but logs an unhandled rejection: `AiApps.scan` reads `response.value.apps` from a null mock (`apps.svelte.ts:47`). Comes from T-0008, not this batch.

## Blocked

## Done

- [x] T-0044 · Calendar: reminders as Windows notifications · area: desktop/src/focus_alerts.rs, crates/zephium-calendar · owner: orca w3-reminders · updated: 2026-11-13
  - `677d62183`, `bfea83a50`, `cf7d5820e`: reminder plan + fired-reminder ledger in `zephium-calendar`, desktop scheduler, toast via `focus_alerts` (title + start + short location), switches `calendar.reminders` and `calendar.default_reminder`. clippy clean; calendar 108, preferences 19, focus_alerts 7 tests, frame typecheck/lint/build/settings unit tests pass (Windows). Limits: no native toast observed, scheduler loop not unit-tested, settings changes apply within 60 s.
- [x] T-0046 · Calendar: links with Time, Tasks and meeting links · area: frame/src/features/time, frame/src/features/tasks, frame/src/features/calendar · owner: orca w2-calendar-ui · updated: 2026-11-13
  - Today's events in the Time panel / new tab; link events to Tasks; open Teams/Meet/Zoom links as tabs.
  - `46c4b2000`, `abc36b826`: today's events in the Time panel and new tab, "Create task" from an event (link back via reserved address `calendar.zephium.invalid` in the task context), calendar mark on tasks, Join within 10 min. Frame gates + 31 component tests + calendar domain unit tests pass (Windows). Tasks.svelte JS budget +110 bytes (lazy import made it larger). Limits: hidden when time tracking is off or in private windows; a second task per event possible after reload.
- [x] T-0024 · Mail: several accounts per profile · area: zephium-store, frame/src/features/mail · owner: orca w3-mail-followups · updated: 2026-11-13
  - Display name, colour, signature; reorder and remove accounts.
  - 2026-11-13: several accounts, display name/colour/signature/remove done in the UI; reorder missing (no backend command). Wave 3.
  - Mail follow-ups (merged `d82c6b03e`): clippy mail/desktop clean, zephium-mail 50 unit + 20 flow, desktop mail/oauth 19, frame gates, 77 mail unit + 31 component tests, build within budgets (Windows). Account order via `mail_set_account_order` (app setting `mail.account_order`).
- [x] T-0028 · Mail: safe reading pane · area: frame/src/features/mail, desktop (locked-down view) · owner: orca w3-mail-followups · updated: 2026-11-13
  - Text and HTML with no scripts; remote images blocked until "Load images"; links open as browser tabs and show their real target; attachments save/open. Follow docs/security-model.md.
  - 2026-11-13: sanitiser + sandboxed srcdoc iframe with CSP done, links listed below the message (inert inside the iframe). Open: "Load images" cannot work because the frame CSP has no https: in img-src (fix: fetch through the backend into data: URLs, no CSP change); verify the srcdoc iframe renders natively. Wave 3.
  - Mail follow-ups (merged `d82c6b03e`): clippy mail/desktop clean, zephium-mail 50 unit + 20 flow, desktop mail/oauth 19, frame gates, 77 mail unit + 31 component tests, build within budgets (Windows). "Load images" fetches https images through the backend (`mail_fetch_remote_images`, public addresses only, bounded) into data: URLs, CSP unchanged; links stay listed below the message with their real target. `about:srcdoc` is not governed by `frame-src` per spec; native WebView2 rendering still to confirm at the final launch.
- [x] T-0034 · Mail: Microsoft 365 / Gmail sign-in (OAuth2, XOAUTH2) · area: crates/zephium-mail · owner: orca w3-mail-followups · updated: 2026-11-13
  - Needs app registrations with Microsoft and Google (client IDs), done by the user. Only if T-0019 shows it is needed, or for personal accounts.
  - 2026-11-13: backend done (`87203452b`..`87d55badd`): `zephium-oauth` (PKCE, loopback redirect, refresh tokens + optional user-pasted client secret only in the OS vault), XOAUTH2 for IMAP/SMTP, IPC `oauth_sign_in`/`oauth_cancel_sign_in`/`oauth_discard_grant`/`oauth_sign_out`, guide `docs/fork-oauth.md`. Tests: oauth 38, mail 53, desktop 13 pass; clippy clean (Windows, fakes only). Open: the sign-in fields in the Mail account setup UI (wave 3, after mail-ui).
  - Mail follow-ups (merged `d82c6b03e`): clippy mail/desktop clean, zephium-mail 50 unit + 20 flow, desktop mail/oauth 19, frame gates, 77 mail unit + 31 component tests, build within budgets (Windows). Sign-in fields (Microsoft/Google, client ID, tenant, optional secret) in account setup; `oauth_sign_in` takes `purposes`. Real provider sign-in not tested.
- [x] T-0033 · Mail: new-mail notifications and unread badge · area: desktop/src/focus_alerts.rs, frame/src/features/sidebar · owner: orca w3-mail-notify · updated: 2026-11-13
  - Windows toast; unread count on the Mail mode button.
  - `85b6fcfcf`, `0783cfaaa`, `17d3f12bf` + app-level hold in `BrowserApp.svelte`: count-only toast via `focus_alerts` (no sender/subject/body), switch `mail.notifications`, unread badge on ModeTabs/ModeWord. clippy, focus_alerts 6 + preferences 19 tests, frame gates, 69 unit + 49 component tests pass (Windows). Not done: clicking the toast does not open Mail (no activation path). Not observed natively.
- [x] T-0015 · Modes: make Work-only Rust branches mode-aware · area: crates/zephium-app/src/shell/{spaces,operations,time,media,browser_pages}.rs · owner: orca w1-modes + w3-time-modes · updated: 2026-11-13
  - Switching space keeps the mode, typing an address returns to Browse, time tracking records the mode. Update the Rust tests in `shell/tests`.
  - 2026-11-13: `5a86ca30c` made spaces/operations/browser_pages mode-aware. Open: time tracking for Mail/Calendar (`Place` only has Site/Work; needs a store change), and media/space/address branches treat every mode like Work.
  - `284890e65`, `661d7fc58`, `132ea35bd`: `Place::Mail`/`Calendar` stored as reserved keys `@mail`/`@calendar` (no migration, old rows load); media show-tab and delete-space kept Work-only, switch_space/typed address shared. clippy clean; core+store time tests 39, app shell tests 766 pass (1 load timeout passes alone); frame typecheck/lint/unit pass (Windows).
- [x] T-0043 · Calendar: create, edit, move and delete events · area: frame/src/features/calendar, crates/zephium-calendar · owner: orca w2-calendar-ui + w2-calendar-store · updated: 2026-11-13
  - Writable CalDAV calendars; title, time, all-day, location, notes, recurrence, reminders; drag to move.
  - 2026-11-13: UI for create/edit/move/delete done (`6727a4639`); recurring events are view/delete-only until the backend exposes series info and `calendar_update_occurrence` (calendar-store worker). All-day banner drag not done.
  - 2026-11-13: backend done (`bd4f38c12`, `b57438c3d`): `calendar_update_occurrence`/`calendar_delete_occurrence`, overrides/EXDATEs kept, VTIMEZONE per TZID, series fields + `CalendarRecurrenceV1` on occurrences. Open: wire the UI to them (recurring edit), all-day drag.
  - `e92295f2b`: "This event / All events" for edit, move, delete; structured recurrence editor; all-day banner drag. Frame typecheck/lint/stylelint/knip/build pass, 375 unit + 20 component tests (Chromium, Windows). Limits: monthly/yearly day selectors read-only; not run in the native app.
- [x] T-0041 · Calendar: local cache and sync · area: crates/zephium-store (calendar tables + migration), crates/zephium-calendar · owner: orca w2-calendar-store · updated: 2026-11-13
  - Sync tokens / ETags; periodic and manual refresh.
  - 2026-11-13: in-memory cache done in `zephium-calendar` (`ccf74bf31`); persistent zephium-store tables still open (wave 2).
  - `bf77ef01b`, `bd4f38c12`: META v30 calendar tables (pinned fingerprint, v29→v30 upgrade test), `StoreCalendar` persistence (calendar.json imported once, renamed `.migrated`), sync-token else ETag delta, 5-min due check / 15-min staleness, stopped on shutdown. `cargo test -p zephium-calendar` 68 pass, calendar store + migration tests pass, clippy clean (Windows). 8 store actor tests time out under parallel load and pass alone. No real CalDAV server test.
- [x] T-0022 · Mail: account setup with automatic server lookup · area: crates/zephium-mail, desktop IPC `mail_*`, frame/src/features/mail · owner: orca w2-mail-ui · updated: 2026-11-13
  - Address + password; Mozilla ISPDB / `autoconfig.<domain>` / `.well-known/autoconfig` / SRV lookup; manual host, port and security fallback; "Test connection".
  - 2026-11-13: backend autoconfig (ISPDB, autoconfig.<domain>, .well-known) + `mail_autoconfig`/`mail_test_account` done (`80dc4770c`, `ba6def012`); UI in wave 2.
  - Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app.
- [x] T-0026 · Mail: folder list and combined inbox · area: frame/src/features/mail, crates/zephium-mail · owner: orca w2-mail-ui · updated: 2026-11-13
  - Inbox, Sent, Drafts, Trash, Archive, Junk via SPECIAL-USE; unread counts; "All inboxes".
  - Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app.
- [x] T-0027 · Mail: message list with threads · area: frame/src/features/mail · owner: orca w2-mail-ui · updated: 2026-11-13
  - Sender, subject, date, snippet, unread/flag/attachment marks; threading by `References`/`In-Reply-To`; virtual scrolling (`shared/lib/virtual-window`).
  - Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app.
- [x] T-0030 · Mail: compose, reply, reply all, forward · area: frame/src/features/mail · owner: orca w2-mail-ui · updated: 2026-11-13
  - Plain text first, simple rich text later; attachments; signatures; "From" picker across accounts; drafts saved to the Drafts folder.
  - Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app.
- [x] T-0031 · Mail: message actions · area: frame/src/features/mail, crates/zephium-mail · owner: orca w2-mail-ui · updated: 2026-11-13
  - Mark read/unread, flag, move/copy (drag to folder), delete to Trash, archive, mark junk; keyboard shortcuts.
  - Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app. Drag-to-folder untested natively.
- [x] T-0032 · Mail: search · area: crates/zephium-mail, zephium-store · owner: orca w2-mail-ui · updated: 2026-11-13
  - Local search of cached mail plus server-side IMAP SEARCH.
  - 2026-11-13: server-side IMAP SEARCH + `mail_search` done (`ba6def012`); UI in wave 2.
  - Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app. Local cache search not added (server search only).
- [x] T-0042 · Calendar: day, week, month and agenda views · area: frame/src/features/calendar, frame/src/domain/calendar · owner: orca w2-calendar-ui · updated: 2026-11-13
  - Several calendars overlaid with colours and show/hide toggles; Today button; mini month navigator.
  - `795aae1df`, `6727a4639`: day/week/month/agenda, colours, show/hide, Today, mini month, keyboard. Frame typecheck/lint/stylelint/knip/build/check:styles pass, 50 unit + 14 component tests (Chromium, Windows). New lazy budget entry for `CalendarPage` (no existing budget raised). Not run in the native app.
- [x] T-0020 · Mail: crate `zephium-mail` with IMAP over TLS · area: new crates/zephium-mail · owner: orca w2-mail-core · updated: 2026-11-13
  - `async-imap` + rustls; IDLE for push; bounded reads; all server data is untrusted input. Unit tests against a fake IMAP server.
  - `80dc4770c`, `a0a1a8d84`, `21e8684c3`: `zephium-mail` IMAP over rustls, IDLE watch, bounded reads; 15 fake-IMAP-server flow tests pass. clippy -D warnings clean (Windows).
- [x] T-0021 · Mail: MIME parsing · area: crates/zephium-mail · owner: orca w2-mail-core · updated: 2026-11-13
  - `mail-parser`: headers, text, HTML, inline images, attachments, encodings; size limits.
  - `80dc4770c`: MIME via `mail-parser` with size caps; unit tests with `.eml` fixtures pass. clippy -D warnings clean (Windows).
- [x] T-0023 · Mail: passwords in Windows Credential Manager · area: crates/zephium-credentials, crates/zephium-mail · owner: orca w2-mail-core · updated: 2026-11-13
  - Never in the store, logs, diagnostics or IPC.
  - `80dc4770c`: passwords only in the OS vault as `app.zephium.mail|<account id>`, never in store/IPC. clippy -D warnings clean (Windows).
- [x] T-0025 · Mail: local cache in the store · area: crates/zephium-store (new mail tables + migration) · owner: orca w2-mail-core · updated: 2026-11-13
  - Headers, flags, bodies on demand; bounded size; sync state per folder (UIDVALIDITY, MODSEQ/CONDSTORE).
  - `205615d43`, `6f92f9c73`: META migration v29 + pinned fingerprint, mail cache tables; 69 store mail/migration tests pass. clippy -D warnings clean (Windows).
- [x] T-0029 · Mail: sending via SMTP · area: crates/zephium-mail · owner: orca w2-mail-core · updated: 2026-11-13
  - `lettre`, STARTTLS/SSL, auth; store a copy in Sent where the server does not.
  - `80dc4770c`: SMTP via `lettre` transport (rustls), copy to Sent per account setting. Real SMTP/STARTTLS untested (no network by rule). clippy -D warnings clean (Windows).
- [x] T-0018 · Modes: texts and the onboarding mock switch · area: frame/messages (13 locales), frame/src/features/onboarding/components/Window.svelte · owner: orca w1-modes · updated: 2026-11-13
  - `mode_mail`, `mode_calendar`, update `ui_mode_work_hint`; the onboarding window draws a fake two-tab switch.
  - `d1d96ab24` texts in 13 locales; `8742f1cd2` onboarding mock shows Browse/Work/Mail/Calendar. Frame typecheck/lint, onboarding unit 19 + component 9 pass, frame build within unchanged budgets (Windows).
- [x] T-0010 · Modes: define the mode list as data · area: frame/src/domain/surface, frame/src/features/sidebar · owner: orca w1-modes · updated: 2026-11-13
  - `{ page, label, icon, feature }` in order Browse, Work, Mail, Calendar; a mode is hidden when its feature is off. Today "Browse" means no full-window page and "Work" is `BrowserPage::Work`.
  - `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).
- [x] T-0011 · Modes: `ModeTabs` shows N modes · area: frame/src/features/sidebar/components/ModeTabs.svelte · owner: orca w1-modes · updated: 2026-11-13
  - Replace the two-option segmented control with one driven by T-0010; update `ModeTabs.component.test.ts`.
  - `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).
- [x] T-0012 · Modes: `ModeWord` rail switch shows N modes · area: frame/src/features/sidebar/components/ModeWord.svelte · owner: orca w1-modes · updated: 2026-11-13
  - N-cell grid and moving thumb instead of `repeat(2, …)` and `data-work=true/false`.
  - `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).
- [x] T-0013 · Modes: generalise "in Work" into "in a full-window mode" · area: frame/src/app/browser/Shell.svelte, Sidebar.svelte, session/tools.svelte.ts, AddressField, SidebarSearch, BrowserApp · owner: orca w1-modes · updated: 2026-11-13
  - About 60 frontend lines check `inWork` / `=== "work"`. Shared mode behaviour: compact rail, no address field, tabs open over the mode, follows space changes. Keep Work-only parts (canvas, Work pane, tool redirect) for Work. Update `Shell` and `surface` tests.
  - `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).
- [x] T-0014 · Modes: Rust `BrowserPage::Mail` and `BrowserPage::Calendar` · area: crates/zephium-app/src/api.rs, desktop/src/lib.rs (`browser.*` match), crates/zephium-core/src/commands.rs · owner: orca w1-modes · updated: 2026-11-13
  - Variants, `command_id`, `feature()`, `feature_of`, the `browser.*` match, and regenerated bindings.
  - `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).
- [x] T-0016 · Modes: generated native `mode.choose` menu · area: desktop/src/lib.rs (~4024-4044) · owner: orca w1-modes · updated: 2026-11-13
  - Today it is hard-coded to Browse and Work.
  - `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).
- [x] T-0017 · Modes: Mail and Calendar switches in Settings → Features · area: frame/src/features/settings/lib/catalog.ts, frame/src/domain/preferences · owner: orca w1-modes · updated: 2026-11-13
  - `mail.enabled`, `calendar.enabled`; turning one off while in its mode returns to Browse (like `operations.rs` does for Work).
  - `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).
- [x] T-0037 · Calendar: crate and iCalendar parsing · area: new crates/zephium-calendar · owner: orca w3-calendar · updated: 2026-11-13
  - `.ics` events with time zones, all-day events and recurrence (RRULE, EXDATE, overrides), e.g. the `icalendar`/`rrule` crates; bounded input.
  - `ccf74bf31`: new `zephium-calendar` (iCalendar parse/serialize, RRULE/RDATE/EXDATE/overrides, time zones). `cargo test -p zephium-calendar` 48 passed, clippy -D warnings clean (Windows).
- [x] T-0038 · Calendar: CalDAV accounts · area: crates/zephium-calendar, desktop IPC `calendar_*` · owner: orca w3-calendar · updated: 2026-11-13
  - Discovery via `.well-known/caldav` and the email domain; username/password in Credential Manager (shared with T-0023).
  - `ccf74bf31` + `033ea500e`: CalDAV discovery/PROPFIND/REPORT/sync-token/ETag, 12 `calendar_*` IPC commands, `zephium:calendar-changed`. No real-server test; editing one instance of a recurring series not supported yet.
- [x] T-0039 · Calendar: ICS subscription links · area: crates/zephium-calendar · owner: orca w3-calendar · updated: 2026-11-13
  - Read-only calendars from secret addresses (Google, Outlook, others).
  - `ccf74bf31`: ICS/webcal subscriptions (read-only). Tested with fakes only.

- [x] T-0005 · Remove stale feature exceptions `focus`/`activity`/`resources` · area: frame/architecture.config.js · owner: orca w4-stale · updated: 2026-11-13
  - `c7b3ecd28`: dropped the three names. `pnpm --dir frame run lint` exit 0, vitest `architecture.config` 9/9 (Windows).
- [x] T-0019 · Mail: check the company server type in Thunderbird · area: (user) · owner: user · updated: 2026-11-13
  - User confirmed the company mail is IMAP.
- [x] T-0035 · Mail: Exchange support (EWS or Microsoft Graph) · area: crates/zephium-mail · owner: - · updated: 2026-11-13
  - Not needed (IMAP), per T-0019.

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
