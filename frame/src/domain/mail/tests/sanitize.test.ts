import { expect, test } from "vitest";
import {
  inlineImageUrl,
  linkify,
  mailFrameCsp,
  mailFrameDocument,
  safeLinkTarget,
  sanitizeMailHtml,
  sanitizeStyle,
} from "../sanitize";

const clean = (html: string, options = {}) => sanitizeMailHtml(html, options);

test("scripts are removed with their content, in any case or spacing", () => {
  const out = clean("a<script>alert(1)</script>b<SCRIPT src=x></SCRIPT >c<script>never closed");
  expect(out.html).toBe("abc");
  expect(out.html).not.toMatch(/script|alert/iu);
});

test("event handlers and unknown attributes never survive", () => {
  const out = clean(
    '<div onclick="x()" onmouseover=y() id=a class=b data-x=1 style="color:red" title="t">hi</div>' +
      '<img src="data:image/png;base64,AAAA" onerror="x()">',
  );
  expect(out.html).not.toMatch(/\bon\w+=|id=|class=|data-x/iu);
  expect(out.html).toContain('style="color: red"');
  expect(out.html).toContain('title="t"');
});

test("javascript: and data: links lose their target, even when obfuscated", () => {
  const out = clean(
    '<a href="javascript:alert(1)">1</a><a href=" JaVa\tScRiPt:alert(1)">2</a>' +
      '<a href="&#106;avascript:alert(1)">3</a><a href="data:text/html,<script>1</script>">4</a>' +
      '<a href="vbscript:x">5</a><a href="//evil.example/x">6</a><a href="/relative">7</a>',
  );
  expect(out.links).toEqual([]);
  expect(out.html).not.toMatch(/href|javascript|data:/iu);
  expect(out.html).toContain(">1</a>");
});

test("http, https and mailto links are listed, never given an href", () => {
  const out = clean(
    '<a href="https://example.com/a?b=1&amp;c=2" title="https://trusted.example/">pay</a>' +
      '<a href="mailto:me@example.com">mail</a>',
  );
  expect(out.links).toEqual(["https://example.com/a?b=1&c=2", "mailto:me@example.com"]);
  expect(out.html).not.toMatch(/href=/iu);
  // The tooltip is the real target, not what the author wrote.
  expect(out.html).toContain('title="https://example.com/a?b=1&amp;c=2"');
  expect(out.html).not.toContain("trusted.example");
  expect(out.html).toContain('data-zephium-link="0"');
});

test("remote images are removed and counted until the backend has fetched them", () => {
  const html = '<img src="https://tracker.example/p.gif" width=1 height=1 alt="x">';
  const blocked = clean(html);
  expect(blocked.remoteImages).toBe(1);
  expect(blocked.remoteLoaded).toBe(0);
  expect(blocked.remoteUrls).toEqual(["https://tracker.example/p.gif"]);
  expect(blocked.html).not.toContain("tracker.example");
  expect(blocked.html).toContain("data-zephium-blocked");
});

test("a fetched remote image is shown as its data url, never as the remote address", () => {
  const html = '<img src="https://img.example/p.png?a=1&amp;b=2" alt="x">';
  const data = "data:image/png;base64,iVBORw0KGgo=";
  const out = clean(html, { remoteImages: new Map([["https://img.example/p.png?a=1&b=2", data]]) });
  expect(out.remoteLoaded).toBe(1);
  expect(out.html).toContain(`src="${data}"`);
  expect(out.html).not.toContain("img.example");
  expect(out.html).not.toContain("data-zephium-blocked");
});

test("a fetched answer is checked again: svg, other types and oversized data stay blocked", () => {
  const html = '<img src="https://img.example/a">';
  for (const bad of [
    "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=",
    "data:text/html;base64,PGI+",
    "https://img.example/a",
    "javascript:alert(1)",
    `data:image/png;base64,${"A".repeat(3 * 1024 * 1024)}`,
  ]) {
    const out = clean(html, { remoteImages: new Map([["https://img.example/a", bad]]) });
    expect(out.remoteLoaded).toBe(0);
    expect(out.html).toContain("data-zephium-blocked");
    expect(out.html).not.toContain("src=");
  }
});

test("only https images can be fetched: http is counted and stays blocked", () => {
  const out = clean('<img src="http://x.example/a.png"><img src="https://x.example/a.png">', {
    remoteImages: new Map([
      ["http://x.example/a.png", "data:image/png;base64,iVBORw0KGgo="],
      ["https://x.example/a.png", "data:image/png;base64,iVBORw0KGgo="],
    ]),
  });
  expect(out.remoteImages).toBe(2);
  expect(out.remoteLoaded).toBe(1);
  expect(out.remoteUrls).toEqual(["https://x.example/a.png"]);
  expect(out.html).not.toContain("x.example");
});

test("the same image is asked for once", () => {
  const out = clean('<img src="https://x.example/a.png"><img src="https://x.example/a.png">');
  expect(out.remoteImages).toBe(2);
  expect(out.remoteUrls).toEqual(["https://x.example/a.png"]);
});

test("other image sources never load: srcset, background, file, ftp, protocol-relative", () => {
  const out = clean(
    '<img src="//x.example/a.png"><img src="file:///c:/a.png"><img srcset="https://x.example/a.png 2x" src="ftp://x/a">' +
      '<table background="https://x.example/b.png"><tr><td background="https://x.example/c">z</td></tr></table>',
    { remoteImages: new Map([["https://x.example/a.png", "data:image/png;base64,iVBORw0KGgo="]]) },
  );
  expect(out.html).not.toMatch(/x\.example|file:|ftp:|srcset|background=/iu);
  expect(out.remoteUrls).toEqual([]);
});

test("cid images resolve from attachments; unknown ones are dropped", () => {
  const url = inlineImageUrl("image/png", "iVBORw0KGgo=");
  expect(url).toBe("data:image/png;base64,iVBORw0KGgo=");
  const inline = new Map([["logo@x", url!]]);
  const out = clean('<img src="cid:logo@x"><img src="cid:<logo@x>"><img src="cid:other">', {
    inline,
  });
  expect(out.html.match(/src="data:image\/png/gu)).toHaveLength(2);
  expect(out.droppedImages).toBe(1);
});

test("inline images respect type and size caps; svg is refused", () => {
  expect(inlineImageUrl("image/svg+xml", "PHN2Zz48L3N2Zz4=")).toBeNull();
  expect(inlineImageUrl("text/html", "AAAA")).toBeNull();
  expect(inlineImageUrl("image/png", "A".repeat(3 * 1024 * 1024))).toBeNull();
  expect(inlineImageUrl("image/png", "not base64!")).toBeNull();
  const out = clean(
    '<img src="data:image/svg+xml;base64,PHN2Zz48L3N2Zz4="><img src="data:text/html;base64,AAAA">' +
      '<img src="data:image/png;base64,AAAA">',
  );
  expect(out.html.match(/src=/gu)).toHaveLength(1);
  expect(out.html).toContain('src="data:image/png;base64,AAAA"');
});

test("forms and their controls are removed, their text stays", () => {
  const out = clean(
    '<form action="https://evil.example" method=post>Name <input name=a value=b><button>Go</button>' +
      "<select><option>1</option></select><textarea>t</textarea> done</form>",
  );
  expect(out.html).toBe("Name  done");
  expect(out.html).not.toMatch(/form|input|action|evil/iu);
});

test("base, meta refresh, link and head content are removed", () => {
  const out = clean(
    '<html><head><title>T</title><base href="https://evil.example/"><meta http-equiv="refresh" content="0;url=https://evil.example">' +
      '<link rel=stylesheet href="https://evil.example/a.css"></head><body><p>ok</p></body></html>',
  );
  expect(out.html).toBe("<p>ok</p>");
});

test("svg, math, iframe, object, embed and media are removed with content", () => {
  const out = clean(
    '<svg onload="x()"><script>1</script><a xlink:href="javascript:1"><text>t</text></a></svg>' +
      '<math><mi>x</mi></math><iframe src="https://evil.example">fallback</iframe>' +
      '<object data="x">o</object><embed src="x"><video src=x><source src=y></video>after',
  );
  expect(out.html).toBe("after");
});

test("style elements are dropped; inline css loses url(), @import and escapes", () => {
  const out = clean(
    "<style>@import url(https://evil.example/a.css); p{background:url(https://evil.example/b.png)}</style>" +
      '<p style="color: blue; background: url(https://evil.example/x.png); width:expression(alert(1)); ' +
      'background-image:url(&quot;x&quot;); font-family: \\75rl(x); position:fixed; margin: 4px">t</p>',
  );
  expect(out.html).toBe('<p style="color: blue; margin: 4px">t</p>');
  expect(sanitizeStyle("color:red/*x*/;behavior:url(x)")).toBe("color: red");
  expect(sanitizeStyle("display:flex;display:block")).toBe("display: block");
  expect(sanitizeStyle("color:var(--x);width:attr(x)")).toBe("");
});

test("comments, conditional comments, doctype and CDATA are removed", () => {
  const out = clean(
    "<!DOCTYPE html><!-- <script>x</script> --><!--[if mso]><script>1</script><![endif]--><![CDATA[x]]><?xml y?>text",
  );
  expect(out.html).toBe("text");
});

test("text is escaped and stray angle brackets cannot form tags", () => {
  expect(clean("1 < 2 > 0 & 3").html).toBe("1 &lt; 2 &gt; 0 &amp; 3");
  expect(clean("<img src=x onerror=alert(1)").html).toBe("");
  expect(clean('<a href="x').html).toBe("");
  const nested = clean("<scr<script>ipt>alert(1)</scr</script>ipt>");
  expect(nested.html).not.toMatch(/<script|alert\(1\)<\/script/iu);
});

test("attribute values are escaped so they cannot break out", () => {
  const out = clean('<div title="&quot; onmouseover=&quot;x()">t</div>');
  expect(out.html).toBe('<div title="&quot; onmouseover=&quot;x()">t</div>');
  const reparsed = out.html.match(/title="([^"]*)"/u)![1]!;
  expect(reparsed).not.toContain('"');
});

test("markup is balanced and nesting is bounded", () => {
  expect(clean("<b><i>x</b>y").html).toBe("<b><i>x</i></b>y");
  expect(clean("x</div></p>").html).toBe("x");
  const deep = clean("<div>".repeat(500) + "x");
  expect(deep.html.match(/<div>/gu)!.length).toBeLessThanOrEqual(64);
  expect(deep.html.match(/<\/div>/gu)).toHaveLength(deep.html.match(/<div>/gu)!.length);
});

test("allowed layout markup passes through", () => {
  const html =
    '<table width="600" cellpadding="0"><tr><td align="center" bgcolor="#fff"><b>Hi</b><br><font color="red" size="2">x</font></td></tr></table>';
  expect(clean(html).html).toBe(
    '<table width="600" cellpadding="0"><tr><td align="center" bgcolor="#fff"><b>Hi</b><br><font color="red" size="2">x</font></td></tr></table>',
  );
});

test("link targets are validated", () => {
  expect(safeLinkTarget("https://example.com/a b")).toBeNull();
  expect(safeLinkTarget("ftp://example.com")).toBeNull();
  expect(safeLinkTarget("https://exa\u0000mple.com")).toBeNull();
  expect(safeLinkTarget(`https://example.com/${"a".repeat(3000)}`)).toBeNull();
  expect(safeLinkTarget("  https://example.com/ ")).toBe("https://example.com/");
});

test("the frame document carries a CSP that allows no network or script", () => {
  const document = mailFrameDocument("<p>x</p>");
  expect(document).toContain("default-src 'none'; img-src data:; style-src 'unsafe-inline'");
  expect(document).not.toMatch(/<script|<base|<link/iu);
  expect(mailFrameCsp()).toBe("default-src 'none'; img-src data:; style-src 'unsafe-inline'");
  expect(mailFrameCsp()).not.toMatch(/https:|http:|script-src/u);
});

test("plain text is split into text and link segments without markup", () => {
  expect(linkify("see https://example.com/a. and <b>x</b> mailto:a@b.co!")).toEqual([
    { text: "see " },
    { text: "https://example.com/a", url: "https://example.com/a" },
    { text: ". and <b>x</b> " },
    { text: "mailto:a@b.co", url: "mailto:a@b.co" },
    { text: "!" },
  ]);
  expect(linkify("no links")).toEqual([{ text: "no links" }]);
});
