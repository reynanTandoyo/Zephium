import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import "$styles/global.css";
import AccountSetupForm from "../components/AccountSetupForm.svelte";
import { account, failed, ok } from "./fixtures";

const native = vi.hoisted(() => ({
  signIn: vi.fn(),
  cancel: vi.fn(),
  discard: vi.fn(),
  test: vi.fn(),
  add: vi.fn(),
  autoconfig: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    oauthSignIn: native.signIn,
    oauthCancelSignIn: native.cancel,
    oauthDiscardGrant: native.discard,
    mailTestAccount: native.test,
    mailAddAccount: native.add,
    mailAutoconfig: native.autoconfig,
  });
});

afterEach(() => {
  for (const mock of Object.values(native)) mock.mockReset();
});

const grant = (over = {}) => ({
  handle: "grant-1",
  provider: "google" as const,
  client_id: "id.apps.googleusercontent.com",
  tenant: null,
  expires_in_secs: 600,
  ...over,
});

test("Microsoft sign-in has a client ID and tenant, no password and no server fields", async () => {
  render(AccountSetupForm, { onadded: vi.fn() });
  await page.getByRole("button", { name: "Microsoft 365 / Outlook" }).click();
  await expect.element(page.getByLabelText("Client ID")).toBeVisible();
  await expect.element(page.getByLabelText("Tenant (optional)")).toBeVisible();
  await expect.element(page.getByLabelText("Password", { exact: true })).not.toBeInTheDocument();
  await expect.element(page.getByLabelText("Client secret (optional)")).not.toBeInTheDocument();
  await expect.element(page.getByText("Incoming server (IMAP)")).not.toBeInTheDocument();
  // The guide is named, and no client ID is shipped.
  await expect.element(page.getByText(/docs\/fork-oauth\.md/u)).toBeVisible();
  await expect.element(page.getByLabelText("Client ID")).toHaveValue("");
  await expect.element(page.getByRole("button", { name: "Sign in with Microsoft" })).toBeDisabled();
  await page.getByLabelText("Email address").fill("me@contoso.com");
  await page.getByLabelText("Client ID").fill("abc-123");
  await expect.element(page.getByRole("button", { name: "Sign in with Microsoft" })).toBeEnabled();
});

test("signing in, testing and adding goes through the grant and never a password", async () => {
  const added = vi.fn();
  native.signIn.mockResolvedValue(ok(grant({ provider: "microsoft", tenant: "contoso" })));
  native.test.mockResolvedValue(ok({ imap: null, smtp: null }));
  native.add.mockResolvedValue(ok(account));
  render(AccountSetupForm, { onadded: added });
  await page.getByRole("button", { name: "Microsoft 365 / Outlook" }).click();
  await page.getByLabelText("Email address").fill("me@contoso.com");
  await page.getByLabelText("Client ID").fill("abc-123");
  await page.getByLabelText("Tenant (optional)").fill("contoso");
  await expect.element(page.getByRole("button", { name: "Add account" })).toBeDisabled();
  await page.getByRole("button", { name: "Sign in with Microsoft" }).click();
  expect(native.signIn).toHaveBeenCalledExactlyOnceWith("microsoft", "abc-123", "contoso", null, [
    "mail",
  ]);
  await expect.element(page.getByText(/Your password never passed through Zephium/u)).toBeVisible();
  await expect.element(page.getByText("Both servers accepted the connection.")).toBeVisible();
  await page.getByRole("button", { name: "Add account" }).click();
  expect(native.add.mock.calls[0]![0]).toMatchObject({
    email: "me@contoso.com",
    password: "",
    oauth_grant: "grant-1",
    imap: { host: "outlook.office365.com" },
  });
  await vi.waitFor(() => expect(added).toHaveBeenCalledWith(account));
});

test("a Google client secret is never echoed and is cleared once the sign-in starts", async () => {
  native.signIn.mockResolvedValue(failed("rejected"));
  render(AccountSetupForm, { onadded: vi.fn() });
  await page.getByRole("button", { name: "Google / Gmail" }).click();
  const secret = page.getByLabelText("Client secret (optional)");
  await expect.element(secret).toHaveAttribute("type", "password");
  await expect.element(page.getByLabelText("Tenant (optional)")).not.toBeInTheDocument();
  await page.getByLabelText("Email address").fill("me@gmail.com");
  await page.getByLabelText("Client ID").fill("id.apps.googleusercontent.com");
  await secret.fill("top-secret");
  await page.getByRole("button", { name: "Sign in with Google" }).click();
  expect(native.signIn).toHaveBeenCalledWith(
    "google",
    "id.apps.googleusercontent.com",
    null,
    "top-secret",
    ["mail"],
  );
  await expect.element(secret).toHaveValue("");
  await expect.element(page.getByText(/The secret was cleared/u)).toBeVisible();
  await expect.element(page.getByText(/rejected this client ID or secret/u)).toBeVisible();
  expect(document.body.textContent).not.toContain("top-secret");
  // Nothing was tested or added: there was no sign-in.
  expect(native.test).not.toHaveBeenCalled();
  expect(native.add).not.toHaveBeenCalled();
});

test("a waiting sign-in can be cancelled, and cancelling shows no error", async () => {
  let finish: (value: unknown) => void = () => {};
  native.signIn.mockReturnValue(new Promise((resolve) => (finish = resolve)));
  native.cancel.mockResolvedValue(ok(null));
  render(AccountSetupForm, { onadded: vi.fn() });
  await page.getByRole("button", { name: "Google / Gmail" }).click();
  await page.getByLabelText("Email address").fill("me@gmail.com");
  await page.getByLabelText("Client ID").fill("id");
  await page.getByRole("button", { name: "Sign in with Google" }).click();
  await expect.element(page.getByRole("button", { name: "Cancel sign-in" })).toBeVisible();
  await page.getByRole("button", { name: "Cancel sign-in" }).click();
  expect(native.cancel).toHaveBeenCalledTimes(1);
  finish(failed("cancelled"));
  await expect.element(page.getByRole("button", { name: "Sign in with Google" })).toBeEnabled();
  await expect.element(page.getByRole("alert")).not.toBeInTheDocument();
});

test("switching back to a password discards the sign-in and restores the password form", async () => {
  native.signIn.mockResolvedValue(ok(grant()));
  native.test.mockResolvedValue(ok({ imap: null, smtp: null }));
  native.discard.mockResolvedValue(ok(null));
  render(AccountSetupForm, { onadded: vi.fn() });
  await page.getByRole("button", { name: "Google / Gmail" }).click();
  await page.getByLabelText("Email address").fill("me@gmail.com");
  await page.getByLabelText("Client ID").fill("id");
  await page.getByRole("button", { name: "Sign in with Google" }).click();
  await expect.element(page.getByText(/Your password never passed through Zephium/u)).toBeVisible();
  await page.getByRole("button", { name: "Password", exact: true }).click();
  expect(native.discard).toHaveBeenCalledExactlyOnceWith("grant-1");
  await expect.element(page.getByLabelText("Password", { exact: true })).toBeVisible();
  await expect.element(page.getByLabelText("Client ID")).not.toBeInTheDocument();
});
