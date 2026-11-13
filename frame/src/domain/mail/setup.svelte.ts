import { commands } from "$shared/ipc/bindings";
import type {
  MailAccountV1,
  MailAutoconfigV1,
  MailEndpointV1,
  MailNewAccountV1,
  MailSecurityV1,
  MailTestV1,
  OAuthGrantV1,
  OAuthProviderV1,
} from "$shared/ipc/bindings";
import { isEmailAddress } from "./address";
import {
  callMail,
  callOAuth,
  READ_TIMEOUT,
  SIGN_IN_TIMEOUT,
  SLOW_TIMEOUT,
  type MailFailure,
  type OAuthFailure,
} from "./call";

/**
 * Where the form is: typing, looking servers up, checking them, signing in at
 * the provider's page in the system browser, or adding.
 */
export type SetupPhase = "form" | "detecting" | "testing" | "adding" | "signing_in";

type EndpointForm = { host: string; port: string; security: MailSecurityV1 };

/** The fixed mail servers of the two sign-in providers. */
const PROVIDER_SERVERS: Record<OAuthProviderV1, { imap: EndpointForm; smtp: EndpointForm }> = {
  microsoft: {
    imap: { host: "outlook.office365.com", port: "993", security: "tls" },
    smtp: { host: "smtp.office365.com", port: "587", security: "start_tls" },
  },
  google: {
    imap: { host: "imap.gmail.com", port: "993", security: "tls" },
    smtp: { host: "smtp.gmail.com", port: "465", security: "tls" },
  },
};

const DEFAULT_COLOURS = ["#3b6fd4", "#2f9e6e", "#c2573a", "#8a5bd6", "#b8860b", "#c2417b"];

const endpointOf = (form: EndpointForm): MailEndpointV1 | null => {
  const port = Number(form.port);
  const host = form.host.trim();
  if (!host || !Number.isInteger(port) || port < 1 || port > 65535) return null;
  return { host, port, security: form.security };
};

const formOf = (endpoint: MailEndpointV1 | null, fallback: EndpointForm): EndpointForm =>
  endpoint
    ? { host: endpoint.host, port: String(endpoint.port), security: endpoint.security }
    : fallback;

/**
 * Adding one account: address and password (or a sign-in with Microsoft or
 * Google through the system browser), servers found or typed, a test
 * connection, then the account. The password and a client secret live here
 * only until they are submitted; they are never read back from native code.
 * A sign-in grant is an opaque, single-use handle: adding the account uses
 * it, and it is thrown away if the form is left first.
 */
export class AccountSetup {
  email = $state("");
  /** Cleared the moment the account is submitted, whatever the outcome. */
  password = $state("");
  displayName = $state("");
  username = $state("");
  colour = $state(DEFAULT_COLOURS[0]!);
  signature = $state("");
  imap = $state<EndpointForm>({ host: "", port: "993", security: "tls" });
  smtp = $state<EndpointForm>({ host: "", port: "465", security: "tls" });
  /** The servers were looked up (true) or are being typed by hand (false). */
  detected = $state<MailAutoconfigV1 | null>(null);
  manual = $state(false);
  phase = $state<SetupPhase>("form");
  failure = $state<MailFailure | null>(null);
  /** The last test, per server; null until tested, cleared when anything changes. */
  test = $state<MailTestV1 | null>(null);
  /** The password was cleared by a submit, so the person must type it again. */
  passwordCleared = $state(false);
  /** Signing in with this provider instead of a password; `null` for a password. */
  oauthProvider = $state<OAuthProviderV1 | null>(null);
  /** The client ID the person registered (not a secret). */
  clientId = $state("");
  /** Microsoft only: `common` when empty. */
  tenant = $state("");
  /** Google "Desktop app" clients only. Cleared the moment a sign-in starts. */
  clientSecret = $state("");
  /** The secret was cleared by a sign-in, so it must be typed again to retry. */
  secretCleared = $state(false);
  /** The finished sign-in; its handle is what adds the account. */
  grant = $state.raw<OAuthGrantV1 | null>(null);
  oauthFailure = $state<OAuthFailure | null>(null);
  private generation = 0;
  private lifetime = new AbortController();

  constructor(seedColour = 0) {
    this.colour = DEFAULT_COLOURS[seedColour % DEFAULT_COLOURS.length]!;
  }

  get busy() {
    return this.phase !== "form";
  }
  get emailValid() {
    return isEmailAddress(this.email.trim());
  }
  /** Servers are known (found or typed) and complete. */
  get serversReady() {
    return endpointOf(this.imap) !== null && endpointOf(this.smtp) !== null;
  }
  get tested() {
    return this.test !== null && this.test.imap === null && this.test.smtp === null;
  }
  get canDetect() {
    return this.emailValid && !this.busy;
  }
  /** There is something to sign in with: a typed password or a finished sign-in. */
  get credentialReady() {
    return this.oauthProvider === null ? this.password.length > 0 : this.grant !== null;
  }
  get canSignIn() {
    return (
      this.oauthProvider !== null &&
      this.emailValid &&
      this.clientId.trim().length > 0 &&
      !this.busy
    );
  }
  get canTest() {
    return this.emailValid && this.credentialReady && this.serversReady && !this.busy;
  }
  get canAdd() {
    return this.canTest && this.tested;
  }

  /** Any change invalidates the last test. */
  touched() {
    this.test = null;
    this.failure = null;
  }

  /**
   * Picks Microsoft or Google sign-in (their servers are fixed), or `null` to
   * go back to a password. Any earlier sign-in is thrown away.
   */
  chooseProvider(provider: OAuthProviderV1 | null) {
    if (this.busy) return;
    this.discardGrant();
    this.oauthProvider = provider;
    this.oauthFailure = null;
    this.secretCleared = false;
    this.clientSecret = "";
    this.touched();
    if (provider) {
      this.imap = { ...PROVIDER_SERVERS[provider].imap };
      this.smtp = { ...PROVIDER_SERVERS[provider].smtp };
      this.manual = false;
    }
  }

  /** Throws a finished sign-in away (best effort: it also expires on its own). */
  private discardGrant() {
    const grant = this.grant;
    if (!grant) return;
    this.grant = null;
    try {
      void Promise.resolve(commands.oauthDiscardGrant(grant.handle)).catch(() => {});
    } catch {
      // Nothing to do: the handle expires after ten minutes.
    }
  }

  /**
   * Opens the provider's page in the system browser and waits for the person
   * to finish there. The password is never typed into Zephium. A client secret
   * is handed over and cleared at once, whatever the outcome.
   */
  async signIn() {
    const provider = this.oauthProvider;
    if (!provider || !this.canSignIn) return;
    this.discardGrant();
    const secret = this.clientSecret.trim();
    const request = {
      clientId: this.clientId.trim(),
      tenant: provider === "microsoft" ? this.tenant.trim() || null : null,
      secret: provider === "google" && secret ? secret : null,
    };
    this.clientSecret = "";
    this.secretCleared = secret.length > 0;
    this.phase = "signing_in";
    this.failure = null;
    this.oauthFailure = null;
    this.test = null;
    const generation = this.generation;
    const outcome = await callOAuth(
      () =>
        commands.oauthSignIn(provider, request.clientId, request.tenant, request.secret, ["mail"]),
      SIGN_IN_TIMEOUT,
      this.lifetime.signal,
    );
    request.secret = null;
    if (generation !== this.generation || outcome.ok === "aborted") return;
    this.phase = "form";
    if (!outcome.ok) {
      // Giving up is not a failure to report.
      if (outcome.error !== "cancelled") this.oauthFailure = outcome.error;
      return;
    }
    this.grant = outcome.value;
    // Both servers are tried with the new access token before anything is stored.
    await this.runTest();
  }

  /** Stops a sign-in that is waiting for the browser; `signIn` then settles as cancelled. */
  cancelSignIn() {
    if (this.phase !== "signing_in") return;
    try {
      void Promise.resolve(commands.oauthCancelSignIn()).catch(() => {});
    } catch {
      // The sign-in ends on its own after three minutes.
    }
  }

  /** Asks the backend for the servers of this address. */
  async detect() {
    if (!this.canDetect) return;
    this.phase = "detecting";
    this.failure = null;
    this.test = null;
    const generation = this.generation;
    const outcome = await callMail(
      () => commands.mailAutoconfig(this.email.trim()),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok === "aborted") return;
    this.phase = "form";
    if (!outcome.ok) {
      this.failure = outcome.error;
      this.manual = true;
      return;
    }
    const found = outcome.value;
    this.detected = found;
    if (found.found && found.imap && found.smtp) {
      this.manual = false;
      this.imap = formOf(found.imap, this.imap);
      this.smtp = formOf(found.smtp, this.smtp);
      this.username = found.username?.trim() || this.email.trim();
    } else {
      // Nothing known for this domain: the person enters the servers.
      this.manual = true;
      this.username ||= this.email.trim();
      const domain = this.email.split("@")[1]?.trim() ?? "";
      if (domain && !this.imap.host) this.imap = { ...this.imap, host: `imap.${domain}` };
      if (domain && !this.smtp.host) this.smtp = { ...this.smtp, host: `smtp.${domain}` };
    }
  }

  enterManually() {
    this.manual = true;
    this.username ||= this.email.trim();
    this.touched();
  }

  private payload(): MailNewAccountV1 | null {
    const imap = endpointOf(this.imap);
    const smtp = endpointOf(this.smtp);
    if (!imap || !smtp) return null;
    const email = this.email.trim();
    return {
      display_name: this.displayName.trim() || email,
      email,
      imap,
      smtp,
      // XOAUTH2 names the mailbox by its address.
      username: this.oauthProvider ? email : this.username.trim() || email,
      colour: this.colour,
      signature: this.signature,
      save_sent_copy: true,
      password: this.password,
      ...(this.grant ? { oauth_grant: this.grant.handle } : {}),
    };
  }

  /** Connects to both servers; nothing is stored. */
  async runTest() {
    if (!this.canTest) return;
    const account = this.payload();
    if (!account) return;
    this.phase = "testing";
    this.failure = null;
    this.test = null;
    const generation = this.generation;
    const outcome = await callMail(
      () => commands.mailTestAccount(account),
      SLOW_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok === "aborted") return;
    this.phase = "form";
    if (!outcome.ok) this.failure = outcome.error;
    else this.test = outcome.value;
  }

  /** Stores the account. The password is cleared here either way. */
  async add(): Promise<MailAccountV1 | null> {
    if (!this.canAdd) return null;
    const account = this.payload();
    if (!account) return null;
    this.phase = "adding";
    this.failure = null;
    const generation = this.generation;
    const outcome = await callMail(
      () => commands.mailAddAccount(account),
      SLOW_TIMEOUT,
      this.lifetime.signal,
    );
    // The password was handed over; it is not kept whatever happens next.
    this.password = "";
    account.password = "";
    // A grant is single-use: the account took it, or it is spent and the person signs in again.
    this.grant = null;
    if (generation !== this.generation || outcome.ok === "aborted") return null;
    this.phase = "form";
    if (!outcome.ok) {
      this.failure = outcome.error;
      this.test = null;
      this.passwordCleared = true;
      return null;
    }
    this.passwordCleared = false;
    return outcome.value;
  }

  dispose() {
    this.cancelSignIn();
    this.generation++;
    this.lifetime.abort();
    this.lifetime = new AbortController();
    this.password = "";
    this.clientSecret = "";
    this.discardGrant();
    this.phase = "form";
  }
}
