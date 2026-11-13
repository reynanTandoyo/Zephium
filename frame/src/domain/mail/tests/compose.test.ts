import { afterEach, expect, test, vi } from "vitest";
import { parseAddresses } from "../address";
import { buildDraft, composeFor, swapSignature, withSignature } from "../compose";
import { account, message } from "./fixtures";

const host = vi.hoisted(() => ({ send: vi.fn(), save: vi.fn() }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ mailSend: host.send, mailSaveDraft: host.save });
});

const { ComposeSession } = await import("../compose.svelte");

const ok = <T>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });

afterEach(() => {
  host.send.mockReset();
  host.save.mockReset();
});

const original = {
  header: message(4, {
    subject: "Plan",
    message_id: "<m4@x>",
    references: ["<m1@x>"],
    from: [{ name: "Ann", email: "ann@x.org" }],
    to: [
      { name: null, email: "me@example.com" },
      { name: null, email: "bob@x.org" },
    ],
    cc: [{ name: null, email: "cy@x.org" }],
  }),
  reply_to: [],
  text: "Line one\r\n\r\nLine two",
  html: null,
  truncated: false,
  attachments: [],
  inline_images: [],
};

test("reply addresses the sender, quotes the text and threads the headers", () => {
  const f = composeFor(
    "reply",
    account("a1", { email: "me@example.com", signature: "Me" }),
    original,
  );
  expect(f.to).toBe("Ann <ann@x.org>");
  expect(f.cc).toBe("");
  expect(f.subject).toBe("Re: Plan");
  expect(f.in_reply_to).toBe("<m4@x>");
  expect(f.references).toEqual(["<m1@x>", "<m4@x>"]);
  expect(f.text).toContain("> Line one\n>\n> Line two");
  expect(f.text.indexOf("-- \nMe")).toBeLessThan(f.text.indexOf("> Line one"));
});

test("reply all adds the others once and never the account itself", () => {
  const f = composeFor("reply_all", account("a1", { email: "ME@example.com" }), original);
  expect(f.to).toBe("Ann <ann@x.org>");
  expect(f.cc).toBe("bob@x.org, cy@x.org");
});

test("a Reply-To header wins, and replying to oneself keeps the recipient", () => {
  const withReplyTo = { ...original, reply_to: [{ name: null, email: "list@x.org" }] };
  expect(composeFor("reply", account("a1"), withReplyTo).to).toBe("list@x.org");
  const own = {
    ...original,
    header: { ...original.header, from: [{ name: null, email: "me@example.com" }] },
  };
  expect(composeFor("reply", account("a1", { email: "me@example.com" }), own).to).toBe(
    "me@example.com",
  );
});

test("forward quotes the original with its headers and has no recipients", () => {
  const f = composeFor("forward", account("a1"), original);
  expect(f.subject).toBe("Fwd: Plan");
  expect(f.to).toBe("");
  expect(f.text).toContain("---------- Forwarded message ----------");
  expect(f.text).toContain("Subject: Plan");
  expect(f.text).toContain("Line two");
  expect(f.in_reply_to).toBeNull();
});

test("signatures are inserted once and swapped with the From account", () => {
  expect(withSignature("hi", "Sig")).toBe("hi\n-- \nSig\n");
  expect(withSignature("hi\n-- \nSig\n", "Sig")).toBe("hi\n-- \nSig\n");
  expect(swapSignature("hi\n-- \nOld\n", "Old", "New")).toBe("hi\n-- \nNew\n");
  expect(swapSignature("hi\n-- \nOld\n", "Old", "")).toBe("hi");
  expect(swapSignature("hi", "", "New")).toBe("hi\n-- \nNew\n");
});

test("recipient fields accept names, quotes, commas and semicolons, and report bad entries", () => {
  expect(parseAddresses('"Doe, Jo" <jo@x.org>; b@x.org, nonsense, c@@x.org')).toEqual({
    addresses: [
      { name: "Doe, Jo", email: "jo@x.org" },
      { name: null, email: "b@x.org" },
    ],
    invalid: ["nonsense", "c@@x.org"],
  });
});

test("a draft needs a recipient to send, valid addresses and attachments within the cap", () => {
  const fields = { ...composeFor("new", account("a1")), to: "a@x.org", subject: "  S " };
  expect(buildDraft({ ...fields, to: "" }, [], [], true)).toMatchObject({
    ok: false,
    problem: "no_recipient",
  });
  expect(buildDraft({ ...fields, to: "" }, [], [], false).ok).toBe(true);
  expect(buildDraft({ ...fields, cc: "bad" }, [], [], true)).toMatchObject({
    ok: false,
    problem: "invalid_address",
    invalid: ["bad"],
  });
  expect(buildDraft(fields, [], [30 * 1024 * 1024], true)).toMatchObject({
    ok: false,
    problem: "too_large",
  });
  const good = buildDraft(fields, [], [], true);
  expect(good.ok && good.draft.subject).toBe("S");
});

const session = () =>
  new ComposeSession(
    {
      ...composeFor("new", account("a1", { signature: "A1" })),
      to: "x@y.org",
      subject: "S",
      text: "body",
    },
    () => [account("a1", { signature: "A1" }), account("a2", { signature: "A2" })],
  );

test("sent is claimed only after the server answered, and a failure keeps the message", async () => {
  const compose = session();
  let release!: (value: unknown) => void;
  host.send.mockReturnValueOnce(new Promise((resolve) => (release = resolve)));
  const sending = compose.send();
  expect(compose.state).toBe("sending");
  release({ status: "ok", data: { sent: true, saved_copy: false } });
  await sending;
  expect(compose.state).toBe("sent");
  expect(compose.savedCopy).toBe(false);

  const failing = session();
  host.send.mockReturnValueOnce(err("auth_failed"));
  expect(await failing.send()).toBe(false);
  expect(failing.state).toBe("failed");
  expect(failing.failure).toBe("auth_failed");
  expect(failing.text).toContain("body");
  failing.edited();
  expect(failing.state).toBe("editing");
});

test("a send that times out is a failure with an unknown outcome, never sent", async () => {
  vi.useFakeTimers();
  const compose = session();
  host.send.mockReturnValueOnce(new Promise(() => {}));
  const sending = compose.send();
  await vi.advanceTimersByTimeAsync(91_000);
  await sending;
  expect(compose.state).toBe("failed");
  expect(compose.unknownOutcome).toBe(true);
  vi.useRealTimers();
});

test("saving a draft marks it saved until the next edit, and refuses invalid recipients", async () => {
  const compose = session();
  host.save.mockReturnValueOnce(ok(null));
  expect(await compose.saveDraft()).toBe(true);
  expect(compose.saved).toBe(true);
  compose.edited();
  expect(compose.saved).toBe(false);
  compose.to = "broken";
  expect(await compose.saveDraft()).toBe(false);
  expect(compose.problem).toBe("invalid_address");
  expect(host.save).toHaveBeenCalledTimes(1);
});

test("changing From swaps the signature; attachments are capped and removable", async () => {
  const compose = session();
  compose.setAccount("a2");
  expect(compose.account).toBe("a2");
  expect(compose.text).toContain("-- \nA2");
  expect(compose.text).not.toContain("A1");
  const file = new File([new Uint8Array([1, 2, 3])], "../evil/a.txt", { type: "text/plain" });
  expect(await compose.attach(file)).toBe(true);
  expect(compose.attachments[0]).toMatchObject({
    filename: ".._evil_a.txt",
    size: 3,
    data_base64: "AQID",
  });
  compose.detach(0);
  expect(compose.attachments).toEqual([]);
  const big = {
    size: 16 * 1024 * 1024,
    name: "b",
    type: "",
    arrayBuffer: () => Promise.reject(new Error("never")),
  } as unknown as File;
  expect(await compose.attach(big)).toBe(false);
  expect(compose.problem).toBe("attachment_too_large");
});
