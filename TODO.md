<!-- todo-format: 1 -->
# TODO

Shared task list for this fork (`reynanTandoyo/Zephium`). People and agents
both read it; every change goes through the `todo` command, which locks the
file, checks it and keeps snapshots. Run `todo help` for the commands.

<!-- todo:begin -->
# Working with TODO.md

- Never edit `TODO.md` (or `TODO-archive.md`) directly; always use `todo`.
- Use one unique agent name per agent via `--agent <name>` or `TODO_AGENT`.
- Start by running `todo summary` or `todo next`.
- Claim a task before working on it (`todo claim <id>`, or `todo next --claim` to pick and claim in one step).
- Exit code `1` means someone else claimed it or the rule refused the change — pick another task.
- Exit code `3` means the lock or lease is busy — try the command again.
- Add notes as you learn things (`todo note <id> "..."`).
- Finish a task after committing: `todo done <id> --note "..." --commit HEAD`.
- Use `todo add --title "..." --priority P2` to record problems you find instead of fixing them silently.
- Use `todo ask "..."` for questions only a person can decide.
- Start every git commit message with the task ID (for example `T-012: short summary`).
<!-- todo:end -->

## Tasks

### T-0002 · Launch `pnpm dev` and confirm "Zephium Dev" starts
- status: open
- owner: —
- priority: P2 · area: desktop, frame
- note (2026-11-13, migrated): Must not touch the installed Zephium or its profile.

### T-0004 · Decide on fork releases for colleague updates
- status: open
- owner: —
- priority: P2 · area: .github, desktop
- note (2026-11-13, migrated): Today the custom updater endpoint points at this fork's releases, and none exist, so no automatic updates happen.
- note (2026-11-13, migrated): Automatic updates would need a minisign key pair, a `pubkey` in `tauri.custom.conf.json` and published `latest.json` and installers.
- note (2026-11-13, migrated): skipped by user for this batch.

### T-0047 · Fix the AI apps mock in settings component tests
- status: open
- owner: —
- priority: P3 · area: frame/src/features/settings/tests, frame/src/domain/ai
- note (2026-11-14, migrated): `FeaturesLook.component.test.ts` passes but logs an unhandled rejection: `AiApps.scan` reads `response.value.apps` from a null mock (`apps.svelte.ts:47`). Comes from T-0008, not this batch.

## Done

### T-0006 · Remove macOS-specific code (Windows-only fork)
- status: dropped
- owner: —
- priority: P2 · area: crates, desktop, frame, xtask, scripts, .github
- note (2026-11-13, migrated): Scope today: 158 macOS-named `.rs` files (about 83k lines), `zephium-webext-macos` (11k) and `zephium-terra-macos-probe` (23k), 1,895 `target_os = "macos"` sites in 188 files, 29 frame files, Swift, plist and macOS CI jobs.
- note (2026-11-13, migrated): Decide the scope first: whole files and crates only, or also the `cfg` blocks. The `cfg` edits in shared files cause conflicts on every upstream merge.
- note (2026-11-13, migrated): Acceptance: Windows `cargo clippy --workspace --all-targets --locked -- -D warnings`, the Windows nextest set and `pnpm --dir frame run check` pass, and `pnpm build:custom` works.
- note (2026-11-16, migrated): skipped by user (keep the macOS code; avoids conflicts when merging upstream).
- note (2026-11-17, coordinator): dropped — skipped by the user on 2026-11-16: keep the macOS code to avoid upstream merge conflicts

### T-0048 · Review round 1 of the mail/calendar batch and its fixes
- status: done
- owner: orca review-1 + fix-mail + fix-calendar
- priority: P2 · area: crates/zephium-{mail,calendar,oauth,store}, desktop, frame mail
- note (2026-11-16, orca review-1 + fix-mail + fix-calendar): Review (claude-fable-5-2), report `C:\Project\zephium-orca\review\review-1-report.md`: blocking B1 invitation settings default (`e00720933`) and B2 sub-hourly RRULE DoS (`15fcb4a30`) fixed; B3 Tasks budget +63 approved by the coordinator.
- note (2026-11-16, orca review-1 + fix-mail + fix-calendar): Fixes: mail N1 watchers after restart, N3 stop on switch-off/shutdown, N5 mailto keeps drafts, N6 IMAP byte limits, N9 no silent expunge, N10 image URL parity, N12 SMTP errors + autoconfig redirects (`88896a479`, `81f9c2bdd`, `099a60c76`); calendar/oauth N2 chunked vault, N3 calendar.enabled gate, N4 META v32 indexes, N7 sender check in Rust, N8 loopback 400, N12 redirect guard (`c37ca947f`..`65088507f`). All with tests, clippy clean (Windows).
- note (2026-11-16, orca review-1 + fix-mail + fix-calendar): Open: mail cache unencrypted in the shared meta database (incl. incognito); META v29–v32 numbering will collide with upstream (plan before the next upstream merge).
- note (2026-11-16, orca review-1 + fix-mail + fix-calendar): Round 2 (report review-2-report.md in the orca review folder): R2-B1 hostile DURATION/TRIGGER panic fixed (`955d496fd`); R2-B2 CalDAV edits dropped unmodelled properties, fixed by patching the stored resource (`c9d2c245b`..`74942cd53`, also floating/unknown TZIDs as local time, split-host CalDAV, VTIMEZONE range, unique calendar keys, vault chunk generations; calendar 151, oauth 47 tests); mail watcher backoff + "Sign-in failed" state and IMAP literal cap (`567803f4b`; mail 108 tests). clippy clean (Windows).
- note (2026-11-16, orca review-1 + fix-mail + fix-calendar): Open (batch paused 2026-11-16 by the user): short review of the round 2 fixes, final fmt/clippy/frame check, `pnpm build:custom`, launch check, T-0002. Known limits: instance-only REQUEST into a stored series rebuilds that override from the model; "Sign-in failed" is in-memory.

### T-0045 · Calendar: meeting invites from Mail
- status: done
- owner: orca w3-invites + w3-mail-extras
- priority: P2 · area: crates/zephium-calendar, crates/zephium-mail
- note (2026-11-13, orca w3-invites + w3-mail-extras): Show `.ics` invitations in Mail; answer Accept / Tentative / Decline (iTIP over SMTP); show attendees and their status.
- note (2026-11-16, orca w3-invites + w3-mail-extras): calendar side done (`8fbe83362`, `6d1d94f1b`): iTIP parse, attendee match, sequence compare, REPLY builder, apply to CalDAV calendars; IPC `calendar_invitation_parse`/`_respond`/`_apply_cancel`/`_apply_reply`. calendar 126 tests pass, clippy clean (Windows). Nothing auto-sent. Open: mail side (detection, From/organizer check, card, sending the REPLY); Google/Microsoft calendars cannot store invited events.
- note (2026-11-13, orca w3-invites + w3-mail-extras): Mail side `2b75ce6d2`: invitation detection, lazy InvitationCard, From must match ORGANIZER (request/cancel) or the attendee (reply) else read-only, reply sent only on click as iMIP `text/calendar; method=REPLY`, settings `calendar.invitation_decline`/`_cancel`. clippy clean; zephium-mail 89, preferences 19, desktop mail 23, frame gates, 127 unit + 52 mail component tests pass (Windows). Limits: no alias addresses, only the first calendar part per message, no live-server interop test. Tasks.svelte budget +63 bytes (shared bindings chunk).

### T-0040 · Calendar: Google Calendar / Microsoft Graph accounts (OAuth)
- status: done
- owner: orca w3-calendar-oauth + w2-calendar-ui
- priority: P2 · area: crates/zephium-calendar
- note (2026-11-13, orca w3-calendar-oauth + w2-calendar-ui): Shares sign-in with T-0034; only if needed (see T-0019).
- note (2026-11-13, orca w3-calendar-oauth + w2-calendar-ui): backend done (`2e9d71508`..`0e618bca7` + META v31 `oauth` account kind): Google Calendar API v3 and Microsoft Graph (calendarView/delta) providers, `calendar_add_oauth_account(grant, name)`, guide updated. Tests: calendar 87, oauth 39, store calendar 9 + migrations 61 pass, clippy clean (Windows, fakes only). Open: `oauth_sign_in` `purposes` argument (mail follow-ups worker), the UI fields in Calendar accounts, occurrence edits on Microsoft unsupported.
- note (2026-11-13, orca w3-calendar-oauth + w2-calendar-ui): UI (merged `541b8e70a`): "Sign in with Microsoft/Google" in Calendar Accounts (client ID, tenant, never-echoed optional secret), `oauth_sign_in` with purposes `[calendar]` → `calendar_add_oauth_account`; AccountsPanel lazy, no budget change. Frame gates, 75 calendar unit + 26 component tests pass (Windows). Real sign-in with Microsoft/Google not tested.

### T-0036 · Mail: extras
- status: done
- owner: orca w3-mail-extras
- priority: P2 · area: crates/zephium-mail, frame/src/features/mail
- note (2026-11-13, orca w3-mail-extras): POP3 (optional), offline mode, rules/filters, contact suggestions from sent mail, `mailto:` links opening Zephium's compose.
- note (2026-11-13, orca w3-mail-extras): `8c6f2159d`..`a1f5706e4`: contact suggestions (`mail_contacts`), `mailto:` from pages opens compose (RFC 6068 parser, narrow hook in the desktop engine event sink, event `zephium:mail-compose`), offline indicator from the cache, per-account rules (`mail.rules`, lazy RuleEditor). clippy clean; zephium-mail 84, store mail 15, desktop mail 22 tests, frame gates, 112 unit + 43 mail component tests, build pass (Windows). Not done: POP3 (optional; does not fit the IMAP-only engine), registering as the Windows default mail app (system setting). Mail budget entry updated (fork-only), RuleEditor entry added.

### T-0044 · Calendar: reminders as Windows notifications
- status: done
- owner: orca w3-reminders
- priority: P2 · area: desktop/src/focus_alerts.rs, crates/zephium-calendar
- note (2026-11-13, orca w3-reminders): `677d62183`, `bfea83a50`, `cf7d5820e`: reminder plan + fired-reminder ledger in `zephium-calendar`, desktop scheduler, toast via `focus_alerts` (title + start + short location), switches `calendar.reminders` and `calendar.default_reminder`. clippy clean; calendar 108, preferences 19, focus_alerts 7 tests, frame typecheck/lint/build/settings unit tests pass (Windows). Limits: no native toast observed, scheduler loop not unit-tested, settings changes apply within 60 s.

### T-0046 · Calendar: links with Time, Tasks and meeting links
- status: done
- owner: orca w2-calendar-ui
- priority: P2 · area: frame/src/features/time, frame/src/features/tasks, frame/src/features/calendar
- note (2026-11-13, orca w2-calendar-ui): Today's events in the Time panel / new tab; link events to Tasks; open Teams/Meet/Zoom links as tabs.
- note (2026-11-13, orca w2-calendar-ui): `46c4b2000`, `abc36b826`: today's events in the Time panel and new tab, "Create task" from an event (link back via reserved address `calendar.zephium.invalid` in the task context), calendar mark on tasks, Join within 10 min. Frame gates + 31 component tests + calendar domain unit tests pass (Windows). Tasks.svelte JS budget +110 bytes (lazy import made it larger). Limits: hidden when time tracking is off or in private windows; a second task per event possible after reload.

### T-0024 · Mail: several accounts per profile
- status: done
- owner: orca w3-mail-followups
- priority: P2 · area: zephium-store, frame/src/features/mail
- note (2026-11-13, orca w3-mail-followups): Display name, colour, signature; reorder and remove accounts.
- note (2026-11-13, orca w3-mail-followups): several accounts, display name/colour/signature/remove done in the UI; reorder missing (no backend command). Wave 3.
- note (2026-11-13, orca w3-mail-followups): Mail follow-ups (merged `d82c6b03e`): clippy mail/desktop clean, zephium-mail 50 unit + 20 flow, desktop mail/oauth 19, frame gates, 77 mail unit + 31 component tests, build within budgets (Windows). Account order via `mail_set_account_order` (app setting `mail.account_order`).

### T-0028 · Mail: safe reading pane
- status: done
- owner: orca w3-mail-followups
- priority: P2 · area: frame/src/features/mail, desktop (locked-down view)
- note (2026-11-13, orca w3-mail-followups): Text and HTML with no scripts; remote images blocked until "Load images"; links open as browser tabs and show their real target; attachments save/open. Follow docs/security-model.md.
- note (2026-11-13, orca w3-mail-followups): sanitiser + sandboxed srcdoc iframe with CSP done, links listed below the message (inert inside the iframe). Open: "Load images" cannot work because the frame CSP has no https: in img-src (fix: fetch through the backend into data: URLs, no CSP change); verify the srcdoc iframe renders natively. Wave 3.
- note (2026-11-13, orca w3-mail-followups): Mail follow-ups (merged `d82c6b03e`): clippy mail/desktop clean, zephium-mail 50 unit + 20 flow, desktop mail/oauth 19, frame gates, 77 mail unit + 31 component tests, build within budgets (Windows). "Load images" fetches https images through the backend (`mail_fetch_remote_images`, public addresses only, bounded) into data: URLs, CSP unchanged; links stay listed below the message with their real target. `about:srcdoc` is not governed by `frame-src` per spec; native WebView2 rendering still to confirm at the final launch.

### T-0034 · Mail: Microsoft 365 / Gmail sign-in (OAuth2, XOAUTH2)
- status: done
- owner: orca w3-mail-followups
- priority: P2 · area: crates/zephium-mail
- note (2026-11-13, orca w3-mail-followups): Needs app registrations with Microsoft and Google (client IDs), done by the user. Only if T-0019 shows it is needed, or for personal accounts.
- note (2026-11-13, orca w3-mail-followups): backend done (`87203452b`..`87d55badd`): `zephium-oauth` (PKCE, loopback redirect, refresh tokens + optional user-pasted client secret only in the OS vault), XOAUTH2 for IMAP/SMTP, IPC `oauth_sign_in`/`oauth_cancel_sign_in`/`oauth_discard_grant`/`oauth_sign_out`, guide `docs/fork-oauth.md`. Tests: oauth 38, mail 53, desktop 13 pass; clippy clean (Windows, fakes only). Open: the sign-in fields in the Mail account setup UI (wave 3, after mail-ui).
- note (2026-11-13, orca w3-mail-followups): Mail follow-ups (merged `d82c6b03e`): clippy mail/desktop clean, zephium-mail 50 unit + 20 flow, desktop mail/oauth 19, frame gates, 77 mail unit + 31 component tests, build within budgets (Windows). Sign-in fields (Microsoft/Google, client ID, tenant, optional secret) in account setup; `oauth_sign_in` takes `purposes`. Real provider sign-in not tested.

### T-0033 · Mail: new-mail notifications and unread badge
- status: done
- owner: orca w3-mail-notify
- priority: P2 · area: desktop/src/focus_alerts.rs, frame/src/features/sidebar
- note (2026-11-13, orca w3-mail-notify): Windows toast; unread count on the Mail mode button.
- note (2026-11-13, orca w3-mail-notify): `85b6fcfcf`, `0783cfaaa`, `17d3f12bf` + app-level hold in `BrowserApp.svelte`: count-only toast via `focus_alerts` (no sender/subject/body), switch `mail.notifications`, unread badge on ModeTabs/ModeWord. clippy, focus_alerts 6 + preferences 19 tests, frame gates, 69 unit + 49 component tests pass (Windows). Not done: clicking the toast does not open Mail (no activation path). Not observed natively.

### T-0015 · Modes: make Work-only Rust branches mode-aware
- status: done
- owner: orca w1-modes + w3-time-modes
- priority: P2 · area: crates/zephium-app/src/shell/{spaces,operations,time,media,browser_pages}.rs
- note (2026-11-13, orca w1-modes + w3-time-modes): Switching space keeps the mode, typing an address returns to Browse, time tracking records the mode. Update the Rust tests in `shell/tests`.
- note (2026-11-13, orca w1-modes + w3-time-modes): `5a86ca30c` made spaces/operations/browser_pages mode-aware. Open: time tracking for Mail/Calendar (`Place` only has Site/Work; needs a store change), and media/space/address branches treat every mode like Work.
- note (2026-11-13, orca w1-modes + w3-time-modes): `284890e65`, `661d7fc58`, `132ea35bd`: `Place::Mail`/`Calendar` stored as reserved keys `@mail`/`@calendar` (no migration, old rows load); media show-tab and delete-space kept Work-only, switch_space/typed address shared. clippy clean; core+store time tests 39, app shell tests 766 pass (1 load timeout passes alone); frame typecheck/lint/unit pass (Windows).

### T-0043 · Calendar: create, edit, move and delete events
- status: done
- owner: orca w2-calendar-ui + w2-calendar-store
- priority: P2 · area: frame/src/features/calendar, crates/zephium-calendar
- note (2026-11-13, orca w2-calendar-ui + w2-calendar-store): Writable CalDAV calendars; title, time, all-day, location, notes, recurrence, reminders; drag to move.
- note (2026-11-13, orca w2-calendar-ui + w2-calendar-store): UI for create/edit/move/delete done (`6727a4639`); recurring events are view/delete-only until the backend exposes series info and `calendar_update_occurrence` (calendar-store worker). All-day banner drag not done.
- note (2026-11-13, orca w2-calendar-ui + w2-calendar-store): backend done (`bd4f38c12`, `b57438c3d`): `calendar_update_occurrence`/`calendar_delete_occurrence`, overrides/EXDATEs kept, VTIMEZONE per TZID, series fields + `CalendarRecurrenceV1` on occurrences. Open: wire the UI to them (recurring edit), all-day drag.
- note (2026-11-13, orca w2-calendar-ui + w2-calendar-store): `e92295f2b`: "This event / All events" for edit, move, delete; structured recurrence editor; all-day banner drag. Frame typecheck/lint/stylelint/knip/build pass, 375 unit + 20 component tests (Chromium, Windows). Limits: monthly/yearly day selectors read-only; not run in the native app.

### T-0041 · Calendar: local cache and sync
- status: done
- owner: orca w2-calendar-store
- priority: P2 · area: crates/zephium-store (calendar tables + migration), crates/zephium-calendar
- note (2026-11-13, orca w2-calendar-store): Sync tokens / ETags; periodic and manual refresh.
- note (2026-11-13, orca w2-calendar-store): in-memory cache done in `zephium-calendar` (`ccf74bf31`); persistent zephium-store tables still open (wave 2).
- note (2026-11-13, orca w2-calendar-store): `bf77ef01b`, `bd4f38c12`: META v30 calendar tables (pinned fingerprint, v29→v30 upgrade test), `StoreCalendar` persistence (calendar.json imported once, renamed `.migrated`), sync-token else ETag delta, 5-min due check / 15-min staleness, stopped on shutdown. `cargo test -p zephium-calendar` 68 pass, calendar store + migration tests pass, clippy clean (Windows). 8 store actor tests time out under parallel load and pass alone. No real CalDAV server test.

### T-0022 · Mail: account setup with automatic server lookup
- status: done
- owner: orca w2-mail-ui
- priority: P2 · area: crates/zephium-mail, desktop IPC `mail_*`, frame/src/features/mail
- note (2026-11-13, orca w2-mail-ui): Address + password; Mozilla ISPDB / `autoconfig.<domain>` / `.well-known/autoconfig` / SRV lookup; manual host, port and security fallback; "Test connection".
- note (2026-11-13, orca w2-mail-ui): backend autoconfig (ISPDB, autoconfig.<domain>, .well-known) + `mail_autoconfig`/`mail_test_account` done (`80dc4770c`, `ba6def012`); UI in wave 2.
- note (2026-11-13, orca w2-mail-ui): Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app.

### T-0026 · Mail: folder list and combined inbox
- status: done
- owner: orca w2-mail-ui
- priority: P2 · area: frame/src/features/mail, crates/zephium-mail
- note (2026-11-13, orca w2-mail-ui): Inbox, Sent, Drafts, Trash, Archive, Junk via SPECIAL-USE; unread counts; "All inboxes".
- note (2026-11-13, orca w2-mail-ui): Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app.

### T-0027 · Mail: message list with threads
- status: done
- owner: orca w2-mail-ui
- priority: P2 · area: frame/src/features/mail
- note (2026-11-13, orca w2-mail-ui): Sender, subject, date, snippet, unread/flag/attachment marks; threading by `References`/`In-Reply-To`; virtual scrolling (`shared/lib/virtual-window`).
- note (2026-11-13, orca w2-mail-ui): Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app.

### T-0030 · Mail: compose, reply, reply all, forward
- status: done
- owner: orca w2-mail-ui
- priority: P2 · area: frame/src/features/mail
- note (2026-11-13, orca w2-mail-ui): Plain text first, simple rich text later; attachments; signatures; "From" picker across accounts; drafts saved to the Drafts folder.
- note (2026-11-13, orca w2-mail-ui): Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app.

### T-0031 · Mail: message actions
- status: done
- owner: orca w2-mail-ui
- priority: P2 · area: frame/src/features/mail, crates/zephium-mail
- note (2026-11-13, orca w2-mail-ui): Mark read/unread, flag, move/copy (drag to folder), delete to Trash, archive, mark junk; keyboard shortcuts.
- note (2026-11-13, orca w2-mail-ui): Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app. Drag-to-folder untested natively.

### T-0032 · Mail: search
- status: done
- owner: orca w2-mail-ui
- priority: P2 · area: crates/zephium-mail, zephium-store
- note (2026-11-13, orca w2-mail-ui): Local search of cached mail plus server-side IMAP SEARCH.
- note (2026-11-13, orca w2-mail-ui): server-side IMAP SEARCH + `mail_search` done (`ba6def012`); UI in wave 2.
- note (2026-11-13, orca w2-mail-ui): Mail UI `eecd02def`, `675346384` (merged `65c2220c5`): frame typecheck/lint/stylelint/knip/build pass, 67 unit + 21 component mail tests (Chromium, Windows). Not run in the native app. Local cache search not added (server search only).

### T-0042 · Calendar: day, week, month and agenda views
- status: done
- owner: orca w2-calendar-ui
- priority: P2 · area: frame/src/features/calendar, frame/src/domain/calendar
- note (2026-11-13, orca w2-calendar-ui): Several calendars overlaid with colours and show/hide toggles; Today button; mini month navigator.
- note (2026-11-13, orca w2-calendar-ui): `795aae1df`, `6727a4639`: day/week/month/agenda, colours, show/hide, Today, mini month, keyboard. Frame typecheck/lint/stylelint/knip/build/check:styles pass, 50 unit + 14 component tests (Chromium, Windows). New lazy budget entry for `CalendarPage` (no existing budget raised). Not run in the native app.

### T-0020 · Mail: crate `zephium-mail` with IMAP over TLS
- status: done
- owner: orca w2-mail-core
- priority: P2 · area: new crates/zephium-mail
- note (2026-11-13, orca w2-mail-core): `async-imap` + rustls; IDLE for push; bounded reads; all server data is untrusted input. Unit tests against a fake IMAP server.
- note (2026-11-13, orca w2-mail-core): `80dc4770c`, `a0a1a8d84`, `21e8684c3`: `zephium-mail` IMAP over rustls, IDLE watch, bounded reads; 15 fake-IMAP-server flow tests pass. clippy -D warnings clean (Windows).

### T-0021 · Mail: MIME parsing
- status: done
- owner: orca w2-mail-core
- priority: P2 · area: crates/zephium-mail
- note (2026-11-13, orca w2-mail-core): `mail-parser`: headers, text, HTML, inline images, attachments, encodings; size limits.
- note (2026-11-13, orca w2-mail-core): `80dc4770c`: MIME via `mail-parser` with size caps; unit tests with `.eml` fixtures pass. clippy -D warnings clean (Windows).

### T-0023 · Mail: passwords in Windows Credential Manager
- status: done
- owner: orca w2-mail-core
- priority: P2 · area: crates/zephium-credentials, crates/zephium-mail
- note (2026-11-13, orca w2-mail-core): Never in the store, logs, diagnostics or IPC.
- note (2026-11-13, orca w2-mail-core): `80dc4770c`: passwords only in the OS vault as `app.zephium.mail|<account id>`, never in store/IPC. clippy -D warnings clean (Windows).

### T-0025 · Mail: local cache in the store
- status: done
- owner: orca w2-mail-core
- priority: P2 · area: crates/zephium-store (new mail tables + migration)
- note (2026-11-13, orca w2-mail-core): Headers, flags, bodies on demand; bounded size; sync state per folder (UIDVALIDITY, MODSEQ/CONDSTORE).
- note (2026-11-13, orca w2-mail-core): `205615d43`, `6f92f9c73`: META migration v29 + pinned fingerprint, mail cache tables; 69 store mail/migration tests pass. clippy -D warnings clean (Windows).

### T-0029 · Mail: sending via SMTP
- status: done
- owner: orca w2-mail-core
- priority: P2 · area: crates/zephium-mail
- note (2026-11-13, orca w2-mail-core): `lettre`, STARTTLS/SSL, auth; store a copy in Sent where the server does not.
- note (2026-11-13, orca w2-mail-core): `80dc4770c`: SMTP via `lettre` transport (rustls), copy to Sent per account setting. Real SMTP/STARTTLS untested (no network by rule). clippy -D warnings clean (Windows).

### T-0018 · Modes: texts and the onboarding mock switch
- status: done
- owner: orca w1-modes
- priority: P2 · area: frame/messages (13 locales), frame/src/features/onboarding/components/Window.svelte
- note (2026-11-13, orca w1-modes): `mode_mail`, `mode_calendar`, update `ui_mode_work_hint`; the onboarding window draws a fake two-tab switch.
- note (2026-11-13, orca w1-modes): `d1d96ab24` texts in 13 locales; `8742f1cd2` onboarding mock shows Browse/Work/Mail/Calendar. Frame typecheck/lint, onboarding unit 19 + component 9 pass, frame build within unchanged budgets (Windows).

### T-0010 · Modes: define the mode list as data
- status: done
- owner: orca w1-modes
- priority: P2 · area: frame/src/domain/surface, frame/src/features/sidebar
- note (2026-11-13, orca w1-modes): `{ page, label, icon, feature }` in order Browse, Work, Mail, Calendar; a mode is hidden when its feature is off. Today "Browse" means no full-window page and "Work" is `BrowserPage::Work`.
- note (2026-11-13, orca w1-modes): `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).

### T-0011 · Modes: `ModeTabs` shows N modes
- status: done
- owner: orca w1-modes
- priority: P2 · area: frame/src/features/sidebar/components/ModeTabs.svelte
- note (2026-11-13, orca w1-modes): Replace the two-option segmented control with one driven by T-0010; update `ModeTabs.component.test.ts`.
- note (2026-11-13, orca w1-modes): `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).

### T-0012 · Modes: `ModeWord` rail switch shows N modes
- status: done
- owner: orca w1-modes
- priority: P2 · area: frame/src/features/sidebar/components/ModeWord.svelte
- note (2026-11-13, orca w1-modes): N-cell grid and moving thumb instead of `repeat(2, …)` and `data-work=true/false`.
- note (2026-11-13, orca w1-modes): `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).

### T-0013 · Modes: generalise "in Work" into "in a full-window mode"
- status: done
- owner: orca w1-modes
- priority: P2 · area: frame/src/app/browser/Shell.svelte, Sidebar.svelte, session/tools.svelte.ts, AddressField, SidebarSearch, BrowserApp
- note (2026-11-13, orca w1-modes): About 60 frontend lines check `inWork` / `=== "work"`. Shared mode behaviour: compact rail, no address field, tabs open over the mode, follows space changes. Keep Work-only parts (canvas, Work pane, tool redirect) for Work. Update `Shell` and `surface` tests.
- note (2026-11-13, orca w1-modes): `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).

### T-0014 · Modes: Rust `BrowserPage::Mail` and `BrowserPage::Calendar`
- status: done
- owner: orca w1-modes
- priority: P2 · area: crates/zephium-app/src/api.rs, desktop/src/lib.rs (`browser.*` match), crates/zephium-core/src/commands.rs
- note (2026-11-13, orca w1-modes): Variants, `command_id`, `feature()`, `feature_of`, the `browser.*` match, and regenerated bindings.
- note (2026-11-13, orca w1-modes): `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).

### T-0016 · Modes: generated native `mode.choose` menu
- status: done
- owner: orca w1-modes
- priority: P2 · area: desktop/src/lib.rs (~4024-4044)
- note (2026-11-13, orca w1-modes): Today it is hard-coded to Browse and Work.
- note (2026-11-13, orca w1-modes): `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).

### T-0017 · Modes: Mail and Calendar switches in Settings → Features
- status: done
- owner: orca w1-modes
- priority: P2 · area: frame/src/features/settings/lib/catalog.ts, frame/src/domain/preferences
- note (2026-11-13, orca w1-modes): `mail.enabled`, `calendar.enabled`; turning one off while in its mode returns to Browse (like `operations.rs` does for Work).
- note (2026-11-13, orca w1-modes): `5a86ca30c` (Rust) + `d1d96ab24` (frame): clippy -D warnings for zephium-app/core/desktop clean, new Rust mode tests pass, frame typecheck/lint/unit (305) pass, frame build within unchanged budgets (Windows).

### T-0037 · Calendar: crate and iCalendar parsing
- status: done
- owner: orca w3-calendar
- priority: P2 · area: new crates/zephium-calendar
- note (2026-11-13, orca w3-calendar): `.ics` events with time zones, all-day events and recurrence (RRULE, EXDATE, overrides), e.g. the `icalendar`/`rrule` crates; bounded input.
- note (2026-11-13, orca w3-calendar): `ccf74bf31`: new `zephium-calendar` (iCalendar parse/serialize, RRULE/RDATE/EXDATE/overrides, time zones). `cargo test -p zephium-calendar` 48 passed, clippy -D warnings clean (Windows).

### T-0038 · Calendar: CalDAV accounts
- status: done
- owner: orca w3-calendar
- priority: P2 · area: crates/zephium-calendar, desktop IPC `calendar_*`
- note (2026-11-13, orca w3-calendar): Discovery via `.well-known/caldav` and the email domain; username/password in Credential Manager (shared with T-0023).
- note (2026-11-13, orca w3-calendar): `ccf74bf31` + `033ea500e`: CalDAV discovery/PROPFIND/REPORT/sync-token/ETag, 12 `calendar_*` IPC commands, `zephium:calendar-changed`. No real-server test; editing one instance of a recurring series not supported yet.

### T-0039 · Calendar: ICS subscription links
- status: done
- owner: orca w3-calendar
- priority: P2 · area: crates/zephium-calendar
- note (2026-11-13, orca w3-calendar): Read-only calendars from secret addresses (Google, Outlook, others).
- note (2026-11-13, orca w3-calendar): `ccf74bf31`: ICS/webcal subscriptions (read-only). Tested with fakes only.

### T-0005 · Remove stale feature exceptions `focus`/`activity`/`resources`
- status: done
- owner: orca w4-stale
- priority: P2 · area: frame/architecture.config.js
- note (2026-11-13, orca w4-stale): `c7b3ecd28`: dropped the three names. `pnpm --dir frame run lint` exit 0, vitest `architecture.config` 9/9 (Windows).

### T-0019 · Mail: check the company server type in Thunderbird
- status: done
- owner: user
- priority: P2 · area: (user)
- note (2026-11-13, user): User confirmed the company mail is IMAP.

### T-0035 · Mail: Exchange support (EWS or Microsoft Graph)
- status: dropped
- owner: —
- priority: P2 · area: crates/zephium-mail
- note (2026-11-13, migrated): Not needed (IMAP), per T-0019.
- note (2026-11-17, coordinator): dropped — not needed: the company mail is IMAP (T-0019)

### T-0008 · Replace built-in AI providers with Claude CLI, Antigravity CLI, OpenCode CLI and a custom BYOK backup
- status: done
- owner: claude-main (+ subagents)
- priority: P2 · area: zephium-agentic, zephium-app work_*, desktop work_*, frame work/assist/settings
- note (2026-11-13, claude-main (+ subagents)): Needs a design decision first: (a) a CLI as the model behind Work's own tool loop, (b) handing whole tasks to the CLI (`work_computer/delegate.rs` already knows Codex and Claude, but it is gated off), or (c) Zephium exposing the browser as an MCP server that the CLI drives. Also decide what happens to the Ask Zephium sidebar and voice dictation.
- note (2026-11-13, claude-main (+ subagents)): Keep the existing OpenAI-compatible "Compatible" BYOK provider (endpoint, model, key) as the backup. Remove the curated providers (Anthropic, OpenAI, Gemini, xAI, Mistral, DeepSeek, Groq, Cerebras, OpenRouter) and Zephium AI.
- note (2026-11-13, claude-main (+ subagents)): Check whether Antigravity has a scriptable CLI before committing to it.
- note (2026-11-13, claude-main (+ subagents)): Design: `docs/fork-cli-models.md`. Branch `cli-models` (from `custom`).
- note (2026-11-13, claude-main (+ subagents)): implemented and committed on `cli-models`. Claude Code and Antigravity are model backends; OpenCode only has detection, Install and Sign in (no client, so it is never usable). The built-in key providers are hidden in the UI, and the custom endpoint stays. Passing: workspace clippy, `cargo test -p zephium-cli-models` (11 unit + 5 fake-app), app/core/agentic/work-composition tests, `pnpm --dir frame run check` (1345 tests), `pnpm build:custom`, the app starts, and a live Claude Code call returned the right tool call in 1.9 s. The user confirmed Ask Zephium works with Claude Code and Antigravity in the app.
- note (2026-11-13, claude-main (+ subagents)): Known: agy saves a conversation file per call under `%USERPROFILE%\.gemini\antigravity-cli\conversations` (no flag to stop it), and an agy call takes about 60 s. Screenshots are not sent to the apps (text only).
- note (2026-11-13, claude-main (+ subagents)): Follow-up fixes: agy's agent allows only its `finish` tool (needed for `--json-schema`), wrapped answers are unwrapped, the model card's `fixed` class no longer collides with Tailwind, each app has its own logo (Claude Code uses the Anthropic mark), and the picker card shows "In use". Merged into `custom` and pushed to `origin/custom`.

### T-0007 · Remove the Zephium account and the hosted "Zephium AI" (Cloud) provider
- status: done
- owner: claude-main (+ subagents)
- priority: P2 · area: zephium-account, desktop/src/account*, Cloud provider code, frame account
- note (2026-11-13, claude-main (+ subagents)): Branch `remove-account` (from `custom`). The account holds only identity, plan, usage, billing and avatar on `api.zephium.app`. Browser settings are local.
- note (2026-11-13, claude-main (+ subagents)): **Keep** `zephium-app` `work_account*` and `WorkOperationV1::PrepareAccount`: these handle *website* account approvals, not the Zephium account.
- note (2026-11-13, claude-main (+ subagents)): Keep the persisted serde variants (`WorkModelProvider::Cloud`, hosted `WorkModelFaultV1`, `PageAssistCostV1::Hosted`) as deprecated and never produced, so saved runs and conversations still load.
- note (2026-11-13, claude-main (+ subagents)): committed on `remove-account`. Passing: `cargo clippy --workspace --all-targets --locked -- -D warnings`, tests for app/agentic/work-composition/page-assist, `pnpm --dir frame run check` (1336 tests), frame build and `check:styles`. 10 `zephium-app` Windows tests (work_files, work_personal, work_lead, export) fail identically on the baseline. Component tests were not run (no browser available).
- note (2026-11-13, claude-main (+ subagents)): the release build launches and shows its window (see T-0009). Merged into `custom` (fast-forward to `beae1f006`) and pushed to `origin/custom`.

### T-0009 · "Zephium Custom" refused to start: "private filesystem path or node is unsafe"
- status: done
- owner: claude-main
- priority: P2 · area: zephium-private-fs, zephium-store
- note (2026-11-13, claude-main): Cause: Windows Work storage accepts only a closed list of app identities, and `app.zephium.custom` was not on it. Added `NativeApplication::Custom` (folder `app.zephium.custom`), its mapping in `WindowsWorkStorage::for_application` and its test entry. An early theory about the `S-1-15-3-…` ACE on `%USERPROFILE%\AppData` was wrong, and that ACE needs no change.
- note (2026-11-13, claude-main): Verified: `cargo test -p zephium-store --features work-execution application_selection`, workspace clippy, `pnpm build:custom`, then a release launch showed the window after 784 ms.

### T-0001 · Build and verify the "Zephium Custom" installer
- status: done
- owner: claude-main
- priority: P2 · area: desktop
- note (2026-11-13, claude-main): `Zephium Custom_1.0.5_x64-setup.exe` built and the release executable starts with its own profile `%APPDATA%\app.zephium.custom`. Not yet installed through the installer.

### T-0003 · Commit the fork files and push branch `custom` to `origin`
- status: done
- owner: claude-main
- priority: P2 · area: repo root
- note (2026-11-13, claude-main): Commit `0564fa74a` on `custom`, pushed to `origin/custom` and tracking it. `main` stays a clean mirror of upstream.
