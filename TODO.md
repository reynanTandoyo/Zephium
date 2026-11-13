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

Last ID: T-0005

## In progress

- [ ] T-0001 · Build and verify the "Zephium Custom" installer · area: desktop · owner: claude-main · updated: 2026-11-13
  - `pnpm build:custom` → `target\release\bundle\nsis\Zephium Custom_<ver>_x64-setup.exe`.
  - Done when it installs next to the official Zephium with its own profile (`app.zephium.custom`) and starts.

## Open

- [ ] T-0002 · Launch `pnpm dev` and confirm "Zephium Dev" starts · area: desktop, frame · owner: - · updated: 2026-11-13
  - Must not touch the installed Zephium or its profile.
- [ ] T-0003 · Push branch `custom` to `origin` · area: repo root · owner: - · updated: 2026-11-13
  - The fork files are committed on the local branch `custom`, and `main` stays a clean mirror of upstream.
  - Push only after the user approves.
- [ ] T-0004 · Decide on fork releases for colleague updates · area: .github, desktop · owner: - · updated: 2026-11-13
  - Today the custom updater endpoint points at this fork's releases, and none exist, so no automatic updates happen.
  - Automatic updates would need a minisign key pair, a `pubkey` in `tauri.custom.conf.json` and published `latest.json` and installers.
- [ ] T-0005 · Remove stale feature exceptions `focus`/`activity`/`resources` · area: frame/architecture.config.js · owner: - · updated: 2026-11-13
  - These folders do not exist in `frame/src/features`. Low priority; consider reporting it upstream instead.

## Blocked

## Done
