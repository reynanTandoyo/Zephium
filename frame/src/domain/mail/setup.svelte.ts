import { commands } from "$shared/ipc/bindings";
import type {
  MailAccountV1,
  MailAutoconfigV1,
  MailEndpointV1,
  MailNewAccountV1,
  MailSecurityV1,
  MailTestV1,
} from "$shared/ipc/bindings";
import { isEmailAddress } from "./address";
import { callMail, READ_TIMEOUT, SLOW_TIMEOUT, type MailFailure } from "./call";

/** Where the form is: typing, looking servers up, checking them, or adding. */
export type SetupPhase = "form" | "detecting" | "testing" | "adding";

export type EndpointForm = { host: string; port: string; security: MailSecurityV1 };

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
 * Adding one account: address and password, servers found or typed, a test
 * connection, then the account. The password lives here only until it is
 * submitted to add the account; it is never read back from native code.
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
  get canTest() {
    return this.emailValid && this.password.length > 0 && this.serversReady && !this.busy;
  }
  get canAdd() {
    return this.canTest && this.tested;
  }

  /** Any change invalidates the last test. */
  touched() {
    this.test = null;
    this.failure = null;
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
      username: this.username.trim() || email,
      colour: this.colour,
      signature: this.signature,
      save_sent_copy: true,
      password: this.password,
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
    this.generation++;
    this.lifetime.abort();
    this.lifetime = new AbortController();
    this.password = "";
    this.phase = "form";
  }
}
