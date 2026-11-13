# Fork guide: sign in to Gmail and Microsoft 365 with OAuth (T-0034)

Status: **backend implemented**, interface pending. Owner: fork
`reynanTandoyo/Zephium`.

This page is for the person who wants to add a **Gmail** or **Microsoft 365 /
Outlook.com** mailbox to Zephium Mail. It is optional. A normal mailbox with an
IMAP server and a password (for example a company mail server) needs none of
this: you type the server, your name and your password in Mail setup.

Gmail and Microsoft no longer let apps sign in to mail with just a password.
They want the app to be *registered*, and Zephium does not ship a registration
of its own (that would put somebody's secret inside the program). So you
register a small app **once, in your own account**, and paste its **client ID**
into Zephium. It takes about ten minutes and costs nothing.

## What stays where

| Thing | Where it lives |
| --- | --- |
| Client ID you paste | Zephium settings (it is not a secret) |
| Your password | Never typed into Zephium: you enter it on the Microsoft or Google page in your normal browser |
| Refresh token (keeps you signed in) | Windows Credential Manager or the macOS Keychain, entry `app.zephium.oauth\|<account id>` |
| Access token (short-lived) | Memory only, gone when Zephium closes |

Nothing secret is written to the database, a log or a diagnostic report.

## A. Microsoft 365 / Outlook.com

You need a Microsoft account that can open the Entra admin centre. A personal
Microsoft account works too (Azure signs you up for free).

1. Open <https://entra.microsoft.com> and sign in.
2. Go to **Identity > Applications > App registrations > New registration**.
3. **Name**: `Zephium Mail` (anything you like).
4. **Supported account types**: choose *Accounts in any organizational
   directory and personal Microsoft accounts* if you want one registration for
   every kind of mailbox. For one company only, choose *this organizational
   directory only* and use the tenant ID in Zephium (see below).
5. **Redirect URI**: choose platform **Public client/native (mobile &
   desktop)** and enter `http://localhost` (exactly this, no port, no path).
   Click **Register**.
6. On the **Overview** page copy the **Application (client) ID**. This is what
   you paste into Zephium. Copy the **Directory (tenant) ID** too if you chose
   a single organisation.
7. Open **Authentication**. Under **Advanced settings** make sure **Allow
   public client flows** is **Yes**, then save.
8. Open **API permissions > Add a permission > APIs my organization uses**,
   search `Office 365 Exchange Online`, choose **Delegated permissions** and tick:
   - `IMAP.AccessAsUser.All`
   - `SMTP.Send`
   Then **Add a permission > Microsoft Graph > Delegated** and tick
   `offline_access`. Click **Add permissions**.
9. Some companies require an administrator to approve these permissions
   (**Grant admin consent**). If Zephium says sign-in was refused, ask your IT
   administrator. Some companies also switch off IMAP or SMTP submission for
   mailboxes; that is a setting IT has to change.

Tenant choices in Zephium: leave the tenant empty (it means `common`, any
account), or type `organizations` (work and school only), `consumers`
(personal only), a tenant ID or a domain such as `contoso.onmicrosoft.com`.

## B. Gmail

1. Open <https://console.cloud.google.com> and create a project (top bar >
   **New project**), for example `Zephium Mail`.
2. Open **APIs & Services > Library**, search **Gmail API** and click
   **Enable**. (Calendar later needs the **Google Calendar API** enabled the
   same way.)
3. Open **APIs & Services > OAuth consent screen** (shown as *Google Auth
   platform* in newer consoles). Choose **External** for a normal Gmail
   account, fill in the app name and your e-mail address, and save.
4. Add the scope `https://mail.google.com/` under **Data access / Scopes**.
5. While the app is in **Testing** status, add your own Gmail address under
   **Audience / Test users**. Only test users can sign in. Google then shows a
   "Google hasn't verified this app" page: choose **Advanced > Go to ... (unsafe)**.
   It is your own app, so this is expected. In Testing status Google expires
   the sign-in after **7 days**; set the status to **In production** (you do not
   need to submit it for review for your own use) to stay signed in longer.
6. Open **APIs & Services > Credentials > Create credentials > OAuth client
   ID**. Application type: **Desktop app**. Name it and click **Create**.
7. Copy the **Client ID** (ends in `.apps.googleusercontent.com`). Google also
   shows a **Client secret** for desktop apps. Google's own token service
   still asks for it, although for a desktop app it is not truly secret. Paste
   it into the optional *Client secret* field in Zephium; Zephium keeps it in
   the OS vault, not in settings.

## C. Where to paste it in Zephium

In **Mail > Add account**, choose **Sign in with Microsoft** or **Sign in with
Google**, paste the **client ID** (and for Google the client secret, for a
single-organisation Microsoft registration the tenant), and click **Sign in**.
Your normal browser opens on the Microsoft or Google page. Approve the access;
the page then says you can close the tab. Zephium finishes the account setup
(IMAP and SMTP servers are filled in for you).

> The interface for this page belongs to the Mail screen and is added after the
> backend. Until then the commands below are the contract.

If you wait more than three minutes, press **Cancel**, or close the tab
without approving, the sign-in ends and nothing is saved.

## D. Calendars (Google Calendar and Microsoft 365)

The same registration can serve calendars; Calendar is optional and a CalDAV
or `.ics` calendar needs none of this. Add the permissions below to the app you
already registered (or register one the same way), then sign in again from the
Calendar account setup, which asks for the calendar permission.

- **Microsoft 365**: in Entra, **API permissions > Add a permission > Microsoft
  Graph > Delegated permissions**, tick `Calendars.ReadWrite` (and
  `offline_access` if it is not there yet). Some organisations need an
  administrator to grant consent. Use `Calendars.ReadWrite` rather than
  read-only so events can be created, moved and deleted.
- **Google**: in the same Cloud project, **APIs & Services > Library**, enable
  the **Google Calendar API** (separate from the Gmail API). Under **Data
  access / Scopes** add `https://www.googleapis.com/auth/calendar`. Testing
  status, test users and the optional client secret work as in section B.

Each calendar account keeps its own copy of the refresh token in the vault
(`app.zephium.oauth|<calendar account id>`); mail and calendar do not share a
token entry, so removing one never signs the other out. One consent can cover
both: the sign-in takes a list of purposes, and Microsoft grants mail and
calendar together (its access tokens are still issued per resource, so the
Graph token comes from a refresh with the calendar scopes).

## Removing access

- In Zephium, removing the mail account deletes its entry from the Credential
  Manager / Keychain.
- To also revoke the app on the provider side: Microsoft at
  <https://myaccount.microsoft.com> (*Apps & services*) or Google at
  <https://myaccount.google.com/permissions>. Deleting the registration in
  Entra or Google Cloud has the same effect for every device.

## How it works (for maintainers)

- Crate `crates/zephium-oauth`: authorization code flow with **PKCE (S256)** for
  a native public client and a **loopback redirect** (RFC 8252). It binds
  `127.0.0.1` on a random port, serves at most eight connections, reads at most
  8 KiB per request within five seconds, accepts only the literal hosts
  `127.0.0.1:<port>` and `localhost:<port>`, and answers the first valid
  redirect. A redirect with a different `state` ends the sign-in. Microsoft
  matches the registered `http://localhost`, so the redirect URI names
  `localhost` for Microsoft and `127.0.0.1` for Google; the listener is the
  same.
- Scopes: Microsoft `https://outlook.office.com/IMAP.AccessAsUser.All
  https://outlook.office.com/SMTP.Send offline_access`; Google
  `https://mail.google.com/`. Calendar scopes (Google
  `https://www.googleapis.com/auth/calendar`, Microsoft Graph
  `Calendars.ReadWrite offline_access`) are `Purpose::Calendar`; the calendar
  service uses the same sign-in through `TokenSessions` (an
  `AccessTokenSource` that also adopts a grant, drops a token after a 401 and
  signs out).
- HTTP goes through the `HttpClient` trait (the desktop supplies `reqwest`,
  HTTPS only, no redirects, 64 KiB cap). Token errors become the closed
  `OAuthError` enum with no server text.
- Mail: `AuthKind` on `zephium_mail::model::Account` is `password` (default,
  so accounts saved earlier still load) or
  `oauth { provider, client_id, tenant? }`. The mail engine asks an
  `AccessTokenSource` for a token whenever it opens an IMAP connection or
  sends, and the `TokenManager` refreshes it from the vault when it is within a
  minute of expiry. IMAP uses `AUTHENTICATE XOAUTH2`, SMTP uses lettre's
  `Mechanism::Xoauth2`.
- Where the auth kind is kept: app setting `mail.auth`, a JSON map of account
  id to `AuthKind` (client ID and tenant, no secrets). The mail cache schema
  is unchanged.

### IPC for the interface

All commands are main-window only.

| Command | Purpose |
| --- | --- |
| `oauth_sign_in(provider, client_id, tenant, client_secret)` | `provider`: `"microsoft"` or `"google"`; `tenant` and `client_secret` may be `null`. Opens the browser and waits up to 3 minutes. Returns `OAuthGrantV1 { handle, provider, client_id, tenant, expires_in_secs }`. One sign-in at a time (`busy`). |
| `oauth_cancel_sign_in()` | Stops the waiting sign-in (`cancelled`). |
| `oauth_discard_grant(handle)` | Drops an unused handle. |
| `oauth_sign_out(account)` | Removes the account's refresh token and cached access token. |
| `mail_test_account({ ..., oauth_grant })` | Tests IMAP and SMTP with the grant's access token instead of a password. |
| `mail_add_account({ ..., password: "", oauth_grant })` | Consumes the handle: the refresh token goes to the vault, the account is created as OAuth. The username is set to the e-mail address. |
| `mail_update_account({ ..., oauth_grant })` | Signs an existing OAuth account in again (handle replaces the refresh token). A `password` on an OAuth account is refused. |

`MailAccountV1` gains an optional `oauth?: { provider, client_id, tenant } | null`,
present only for OAuth accounts. `MailNewAccountV1.oauth_grant` and
`MailAccountUpdateV1.oauth_grant` are optional. Errors use `OAuthErrorV1`
(`invalid_input`, `cancelled`, `timeout`, `offline`, `denied`, `state_mismatch`,
`protocol`, `rejected`, `sign_in_required`, `credentials`, `busy`,
`unsupported`, `internal`). When a refresh token has been revoked, mail
commands fail with `auth_failed`: the interface should offer *Sign in again*.

Handles live in memory only, work once, expire after ten minutes and at most
four are kept.

### Calendars (T-0040)

- Crate `zephium-calendar`, modules `provider/google.rs`, `provider/graph.rs`
  and `service/oauth.rs`. **Google** uses the Calendar API v3 rather than
  Google's CalDAV endpoint: first-class incremental sync (`syncToken`; `410`
  / `fullSyncRequired` makes a full read), JSON, ETag-guarded writes and
  stable instance ids (`<series>_<UTC start>`) for single-occurrence edits;
  the RRULE/EXDATE strings map straight onto the iCalendar model. **Microsoft**
  uses `/me/calendars`, `/me/calendars/{id}/calendarView/delta` (window: 180
  days back, 365 ahead, re-read about every 90 days; an expired delta link is
  a full read) and `/me/events`. Graph occurrences are cached as individual
  events (Exchange has already applied exceptions and cancellations), so
  editing or deleting one changes that occurrence; the series pattern
  (`patternedRecurrence` to RRULE and back) is used when creating a series and
  when a series master is read. Editing a whole Microsoft series is not
  offered; do it in Outlook.
- Bounds: 250 events per Google page and 100 per Graph page, 200 pages,
  20 000 events per calendar, 200 calendars per account, 16 MiB per response;
  Graph links are followed only on `graph.microsoft.com` over HTTPS. Errors
  stay in `CalendarError` (`429` is `limit`, a failed token is `unauthorized`).
- A request that answers 401 drops the cached access token and is sent once
  more with a refreshed one.
- The store schema only allows the account kinds `caldav` and `subscription`,
  so a Google or Microsoft account is saved as `caldav` with the marker
  `oauth1<TAB>provider<TAB>client id<TAB>tenant<TAB>address` in the user-name
  column (no secret in it). A store column of its own would need a migration.

IPC (main window only):

| Command | Purpose |
| --- | --- |
| `calendar_add_oauth_account(grant, name)` | `grant` is the handle from `oauth_sign_in`, which must have been asked for the calendar purpose. Consumes the handle, lists the calendars, syncs in the background. `CalendarAccountV1.kind` is `google` or `microsoft` and `oauth` holds `{ client_id, tenant }`. |
| `calendar_remove_account(account)` | Also removes the refresh token from the vault for these accounts. |

The other `calendar_*` commands work unchanged on these accounts.

### Not covered

- No real provider has been contacted in tests: flows run against fakes and a
  local fake IMAP server.
- Microsoft tokens are per resource. The Graph (calendar) token is obtained
  by refreshing the refresh token with `Purpose::Calendar`, which needs
  `Calendars.ReadWrite` consented at sign-in. `oauth_sign_in` currently asks
  for mail scopes only; it needs a `purposes` argument before a calendar
  account can be added.
- Linux has no vault and no browser opener: OAuth commands report
  `unsupported` or fail to open the page there (Linux is not supported).
