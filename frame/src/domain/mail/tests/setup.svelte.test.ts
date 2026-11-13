import { afterEach, expect, test, vi } from "vitest";
import { account } from "./fixtures";

const host = vi.hoisted(() => ({ auto: vi.fn(), test: vi.fn(), add: vi.fn() }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    mailAutoconfig: host.auto,
    mailTestAccount: host.test,
    mailAddAccount: host.add,
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
