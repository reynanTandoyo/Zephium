import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import "$styles/global.css";
import { MailStore } from "$domain/mail";
import MailSettings from "../components/MailSettings.svelte";
import { account, failed, ok } from "./fixtures";

const native = vi.hoisted(() => ({ order: vi.fn(), rules: vi.fn() }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ mailSetAccountOrder: native.order, mailRules: native.rules });
});

afterEach(() => {
  native.order.mockReset();
  native.rules.mockReset();
});

const three = [
  { ...account, id: "a", email: "a@example.com" },
  { ...account, id: "b", email: "b@example.com" },
  { ...account, id: "c", email: "c@example.com" },
];

function shown() {
  native.rules.mockResolvedValue(ok([]));
  const store = new MailStore();
  store.accounts = three;
  render(MailSettings, { store, onadd: vi.fn(), onback: vi.fn() });
  return store;
}

const emails = () =>
  [...document.querySelectorAll("li.account h3")].map((heading) => heading.textContent);

test("accounts move up and down, and the ends cannot move further", async () => {
  native.order.mockResolvedValue(ok(null));
  shown();
  await expect.element(page.getByRole("button", { name: "Move a@example.com up" })).toBeDisabled();
  await expect
    .element(page.getByRole("button", { name: "Move c@example.com down" }))
    .toBeDisabled();
  await page.getByRole("button", { name: "Move c@example.com up" }).click();
  expect(native.order).toHaveBeenCalledExactlyOnceWith(["a", "c", "b"]);
  await vi.waitFor(() =>
    expect(emails()).toEqual(["a@example.com", "c@example.com", "b@example.com"]),
  );
  await page.getByRole("button", { name: "Move a@example.com down" }).click();
  expect(native.order).toHaveBeenLastCalledWith(["c", "a", "b"]);
  await vi.waitFor(() =>
    expect(emails()).toEqual(["c@example.com", "a@example.com", "b@example.com"]),
  );
});

test("a refused reorder keeps the order and says why", async () => {
  native.order.mockResolvedValue(failed("busy"));
  shown();
  await page.getByRole("button", { name: "Move b@example.com up" }).click();
  await expect.element(page.getByRole("alert")).toHaveTextContent(/busy/iu);
  expect(emails()).toEqual(["a@example.com", "b@example.com", "c@example.com"]);
});

test("an account whose sign-in was refused says so, and the others do not", async () => {
  native.rules.mockResolvedValue(ok([]));
  const store = new MailStore();
  store.accounts = [three[0]!, { ...three[1]!, sign_in_failed: true }];
  render(MailSettings, { store, onadd: vi.fn(), onback: vi.fn() });
  await expect.element(page.getByText(/Sign-in failed/u)).toBeVisible();
  expect(document.querySelectorAll(".signin")).toHaveLength(1);
  // Let the rule lists finish loading before the mocks are reset.
  await vi.waitFor(() => expect(native.rules).toHaveBeenCalled());
});
