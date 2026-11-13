import { afterEach, expect, test, vi } from "vitest";
import { account } from "./fixtures";

const host = vi.hoisted(() => ({
  auto: vi.fn(),
  test: vi.fn(),
  add: vi.fn(),
  signIn: vi.fn(),
  cancel: vi.fn(),
  discard: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    mailAutoconfig: host.auto,
    mailTestAccount: host.test,
    mailAddAccount: host.add,
    oauthSignIn: host.signIn,
    oauthCancelSignIn: host.cancel,
    oauthDiscardGrant: host.discard,
  });
});

const { AccountSetup } = await import("../setup.svelte");

const ok = <T>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });
const endpoint = (name: string, port: number) => ({ host: name, port, security: "tls" as const });

afterEach(() => {
  host.auto.mockReset();
  host.test.mockReset();
  host.add.mockReset();
  host.signIn.mockReset();
  host.cancel.mockReset();
  host.discard.mockReset();
});

const found = () =>
  ok({
    found: true,
    imap: endpoint("imap.example.com", 993),
    smtp: endpoint("smtp.example.com", 465),
    username: "me@example.com",
    provider: "Example",
    source: "ispdb" as const,
  });

test("detecting fills the servers from the lookup", async () => {
  host.auto.mockReturnValue(found());
  const setup = new AccountSetup();
  setup.email = "me@example.com";
  await setup.detect();
  expect(host.auto).toHaveBeenCalledWith("me@example.com");
  expect(setup.imap).toEqual({ host: "imap.example.com", port: "993", security: "tls" });
  expect(setup.smtp.host).toBe("smtp.example.com");
  expect(setup.manual).toBe(false);
  expect(setup.username).toBe("me@example.com");
});

test("an unknown domain or a failed lookup falls back to manual entry", async () => {
  host.auto.mockReturnValueOnce(
    ok({ found: false, imap: null, smtp: null, username: null, provider: null, source: null }),
  );
  const setup = new AccountSetup();
  setup.email = "me@corp.example";
  await setup.detect();
  expect(setup.manual).toBe(true);
  expect(setup.imap.host).toBe("imap.corp.example");
  host.auto.mockReturnValueOnce(err("offline"));
  await setup.detect();
  expect(setup.failure).toBe("offline");
  expect(setup.manual).toBe(true);
});

test("an invalid address is not looked up", async () => {
  const setup = new AccountSetup();
  setup.email = "nope";
  await setup.detect();
  expect(host.auto).not.toHaveBeenCalled();
});

test("adding needs a successful test of the current settings", async () => {
  host.auto.mockReturnValue(found());
  const setup = new AccountSetup();
  setup.email = "me@example.com";
  setup.password = "secret";
  await setup.detect();
  expect(setup.canAdd).toBe(false);
  host.test.mockReturnValueOnce(ok({ imap: null, smtp: "auth_failed" }));
  await setup.runTest();
  expect(setup.test).toEqual({ imap: null, smtp: "auth_failed" });
  expect(setup.canAdd).toBe(false);
  host.test.mockReturnValueOnce(ok({ imap: null, smtp: null }));
  await setup.runTest();
  expect(setup.canAdd).toBe(true);
  setup.imap.host = "other.example.com";
  setup.touched();
  expect(setup.canAdd).toBe(false);
});

test("the password is sent once and cleared after submitting, success or not", async () => {
  host.auto.mockReturnValue(found());
  host.test.mockReturnValue(ok({ imap: null, smtp: null }));
  const setup = new AccountSetup();
  setup.email = "me@example.com";
  setup.password = "secret";
  await setup.detect();
  await setup.runTest();
  const sent: string[] = [];
  host.add.mockImplementationOnce((request: { password: string }) => {
    sent.push(request.password);
    return err("credentials");
  });
  expect(await setup.add()).toBeNull();
  expect(sent).toEqual(["secret"]);
  expect(setup.password).toBe("");
  expect(setup.failure).toBe("credentials");
  expect(setup.passwordCleared).toBe(true);
  expect(setup.canAdd).toBe(false);

  setup.password = "secret2";
  await setup.runTest();
  host.add.mockReturnValueOnce(ok(account("a1")));
  expect((await setup.add())?.id).toBe("a1");
  expect(setup.password).toBe("");
  expect(setup.passwordCleared).toBe(false);
});

test("manual servers need a host and a valid port before they can be tested", () => {
  const setup = new AccountSetup();
  setup.email = "me@example.com";
  setup.password = "x";
  setup.enterManually();
  expect(setup.canTest).toBe(false);
  setup.imap = { host: "imap.example.com", port: "99999", security: "tls" };
  setup.smtp = { host: "smtp.example.com", port: "587", security: "start_tls" };
  expect(setup.canTest).toBe(false);
  setup.imap = { host: "imap.example.com", port: "993", security: "tls" };
  expect(setup.canTest).toBe(true);
});

test("disposing clears a typed password", () => {
  const setup = new AccountSetup();
  setup.password = "secret";
  setup.dispose();
  expect(setup.password).toBe("");
});

const grant = (over = {}) => ({
  handle: "h1",
  provider: "google" as const,
  client_id: "id.apps.googleusercontent.com",
  tenant: null,
  expires_in_secs: 600,
  ...over,
});

function google() {
  const setup = new AccountSetup();
  setup.email = "me@gmail.com";
  setup.chooseProvider("google");
  setup.clientId = " id.apps.googleusercontent.com ";
  return setup;
}

test("choosing a provider fixes its servers and needs no password", () => {
  const setup = new AccountSetup();
  setup.email = "me@outlook.com";
  expect(setup.canSignIn).toBe(false);
  setup.chooseProvider("microsoft");
  expect(setup.imap).toEqual({ host: "outlook.office365.com", port: "993", security: "tls" });
  expect(setup.smtp).toEqual({ host: "smtp.office365.com", port: "587", security: "start_tls" });
  expect(setup.serversReady).toBe(true);
  expect(setup.canSignIn).toBe(false);
  setup.clientId = "abc";
  expect(setup.canSignIn).toBe(true);
  // Signing in is not testing: there is no credential yet.
  expect(setup.canTest).toBe(false);
});

test("signing in sends the typed values, clears the secret at once, then tests with the grant", async () => {
  host.signIn.mockReturnValue(ok(grant()));
  host.test.mockReturnValue(ok({ imap: null, smtp: null }));
  const setup = google();
  setup.clientSecret = "  s3cret  ";
  const done = setup.signIn();
  expect(setup.phase).toBe("signing_in");
  expect(setup.clientSecret).toBe("");
  await done;
  expect(host.signIn).toHaveBeenCalledExactlyOnceWith(
    "google",
    "id.apps.googleusercontent.com",
    null,
    "s3cret",
    ["mail"],
  );
  expect(setup.secretCleared).toBe(true);
  expect(setup.grant?.handle).toBe("h1");
  expect(host.test.mock.calls[0]![0]).toMatchObject({
    email: "me@gmail.com",
    username: "me@gmail.com",
    password: "",
    oauth_grant: "h1",
  });
  expect(setup.canAdd).toBe(true);
});

test("a tenant goes to Microsoft only, and a secret to Google only", async () => {
  host.signIn.mockReturnValue(ok(grant({ provider: "microsoft", tenant: "contoso" })));
  host.test.mockReturnValue(ok({ imap: null, smtp: null }));
  const setup = new AccountSetup();
  setup.email = "me@contoso.com";
  setup.chooseProvider("microsoft");
  setup.clientId = "abc";
  setup.tenant = " contoso ";
  setup.clientSecret = "ignored";
  await setup.signIn();
  expect(host.signIn).toHaveBeenCalledWith("microsoft", "abc", "contoso", null, ["mail"]);
});

test("the account is added with the grant and no password, and the grant is then spent", async () => {
  host.signIn.mockReturnValue(ok(grant()));
  host.test.mockReturnValue(ok({ imap: null, smtp: null }));
  host.add.mockReturnValue(ok(account("a1")));
  const setup = google();
  await setup.signIn();
  expect((await setup.add())?.id).toBe("a1");
  expect(host.add.mock.calls[0]![0]).toMatchObject({ password: "", oauth_grant: "h1" });
  expect(setup.grant).toBeNull();
  expect(host.discard).not.toHaveBeenCalled();
});

test("a refused add spends the grant: signing in again is needed", async () => {
  host.signIn.mockReturnValue(ok(grant()));
  host.test.mockReturnValue(ok({ imap: null, smtp: null }));
  host.add.mockReturnValue(err("busy"));
  const setup = google();
  await setup.signIn();
  expect(await setup.add()).toBeNull();
  expect(setup.failure).toBe("busy");
  expect(setup.grant).toBeNull();
  expect(setup.canTest).toBe(false);
});

test("giving up in the browser is not an error; a refusal is", async () => {
  host.signIn.mockReturnValueOnce(err("cancelled"));
  const setup = google();
  await setup.signIn();
  expect(setup.oauthFailure).toBeNull();
  expect(setup.phase).toBe("form");
  expect(setup.grant).toBeNull();
  host.signIn.mockReturnValueOnce(err("denied"));
  await setup.signIn();
  expect(setup.oauthFailure).toBe("denied");
  expect(host.test).not.toHaveBeenCalled();
});

test("cancelling asks the backend only while a sign-in waits", async () => {
  host.cancel.mockReturnValue(ok(null));
  let finish: (value: unknown) => void = () => {};
  host.signIn.mockReturnValue(new Promise((resolve) => (finish = resolve)));
  const setup = google();
  setup.cancelSignIn();
  expect(host.cancel).not.toHaveBeenCalled();
  const done = setup.signIn();
  setup.cancelSignIn();
  expect(host.cancel).toHaveBeenCalledTimes(1);
  finish(err("cancelled"));
  await done;
  expect(setup.phase).toBe("form");
});

test("an unused grant is discarded when the provider changes or the form is left", async () => {
  host.signIn.mockReturnValue(ok(grant()));
  host.test.mockReturnValue(ok({ imap: null, smtp: null }));
  host.discard.mockReturnValue(ok(null));
  const setup = google();
  await setup.signIn();
  setup.chooseProvider(null);
  expect(host.discard).toHaveBeenCalledExactlyOnceWith("h1");
  expect(setup.grant).toBeNull();
  expect(setup.oauthProvider).toBeNull();

  host.signIn.mockReturnValue(ok(grant({ handle: "h2" })));
  setup.chooseProvider("google");
  setup.clientId = "id";
  await setup.signIn();
  setup.dispose();
  expect(host.discard).toHaveBeenLastCalledWith("h2");
});

test("leaving the form during a sign-in cancels it and wipes the secret", () => {
  host.cancel.mockReturnValue(ok(null));
  host.signIn.mockReturnValue(new Promise(() => {}));
  const setup = google();
  setup.clientSecret = "s";
  void setup.signIn();
  setup.clientSecret = "typed-later";
  setup.dispose();
  expect(host.cancel).toHaveBeenCalledTimes(1);
  expect(setup.clientSecret).toBe("");
});
