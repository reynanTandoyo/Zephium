/**
 * Allow-list sanitiser for received HTML mail.
 *
 * The output is rebuilt from tokens rather than edited in place: only listed
 * tags and attributes are written, every attribute value is re-escaped, and
 * text is escaped. It is the first of three layers; the result is still shown
 * in a sandboxed iframe without scripts, under a CSP that allows no network.
 *
 * Nothing here needs a DOM, so it runs the same in tests and in the frame.
 */

/** Input beyond this is cut; a larger message is shown as truncated text. */
const MAX_HTML_CHARS = 2_000_000;
/** One inline `cid:` or `data:` image may be at most this many decoded bytes. */
const MAX_INLINE_IMAGE_BYTES = 2 * 1024 * 1024;
const MAX_DEPTH = 64;
const MAX_LINKS = 500;
/** Same as `MAX_REMOTE_IMAGES` in `zephium-mail`: the most addresses one request may name. */
const MAX_REMOTE_IMAGES = 24;

const ALLOWED_TAGS = new Set(
  (
    "a abbr address article aside b bdi bdo big blockquote br caption center cite code col colgroup " +
    "dd del dfn div dl dt em figcaption figure font footer h1 h2 h3 h4 h5 h6 header hr i img ins " +
    "kbd li mark nav ol p pre q s samp section small span strike strong sub sup table tbody td " +
    "tfoot th thead tr tt u ul var wbr"
  ).split(" "),
);

/** Elements whose content is dropped together with the tag. */
const DROP_WITH_CONTENT = new Set(
  (
    "script style iframe frame frameset object embed applet noscript noembed noframes template " +
    "title head svg math audio video canvas textarea select option button datalist " +
    "xmp plaintext portal dialog"
  ).split(" "),
);

/** Elements whose content is raw text: the next close tag ends them. */
const RAW_TEXT = new Set(["script", "style", "textarea", "title", "xmp", "plaintext", "noscript"]);

const VOID_TAGS = new Set(["br", "hr", "img", "wbr", "col"]);

const GLOBAL_ATTRIBUTES = new Set(["title", "lang", "dir"]);
const TAG_ATTRIBUTES: Record<string, ReadonlySet<string>> = {
  table: new Set(["width", "height", "border", "cellpadding", "cellspacing", "align", "bgcolor"]),
  td: new Set(["width", "height", "colspan", "rowspan", "align", "valign", "bgcolor"]),
  th: new Set(["width", "height", "colspan", "rowspan", "align", "valign", "bgcolor"]),
  tr: new Set(["align", "valign", "bgcolor"]),
  tbody: new Set(["align", "valign"]),
  thead: new Set(["align", "valign"]),
  tfoot: new Set(["align", "valign"]),
  col: new Set(["width", "span"]),
  colgroup: new Set(["width", "span"]),
  div: new Set(["align"]),
  p: new Set(["align"]),
  h1: new Set(["align"]),
  h2: new Set(["align"]),
  h3: new Set(["align"]),
  h4: new Set(["align"]),
  h5: new Set(["align"]),
  h6: new Set(["align"]),
  font: new Set(["color", "size", "face"]),
  hr: new Set(["width", "size", "align"]),
  ol: new Set(["start", "type"]),
  ul: new Set(["type"]),
  li: new Set(["value"]),
  img: new Set(["alt", "width", "height", "align"]),
  a: new Set([]),
};

const STYLE_PROPERTIES = new Set(
  (
    "color background-color font font-family font-size font-style font-weight font-variant " +
    "line-height text-align text-decoration text-indent text-transform letter-spacing word-spacing " +
    "white-space word-break overflow-wrap vertical-align direction width height max-width min-width " +
    "max-height min-height margin margin-top margin-right margin-bottom margin-left padding " +
    "padding-top padding-right padding-bottom padding-left border border-top border-right " +
    "border-bottom border-left border-color border-style border-width border-collapse border-spacing " +
    "border-radius list-style-type table-layout display"
  ).split(" "),
);
const SAFE_DISPLAY = new Set([
  "block",
  "inline",
  "inline-block",
  "table",
  "table-row",
  "table-cell",
  "none",
]);

const NAMED_ENTITIES: Record<string, string> = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: "\u00a0",
  colon: ":",
  tab: "\t",
  newline: "\n",
  lpar: "(",
  rpar: ")",
};

export type SanitizeOptions = {
  /** Inline images by Content-ID (without angle brackets), as `data:` URLs. */
  inline?: ReadonlyMap<string, string>;
  /**
   * Remote images the backend fetched, by the address the message wrote, as
   * `data:` URLs. Only these are shown: the frame never loads a remote image
   * itself, so its CSP stays closed to the network.
   */
  remoteImages?: ReadonlyMap<string, string>;
};

type SanitizedMail = {
  /** Safe markup for a `<body>`; links carry no `href`. */
  html: string;
  /** Link targets in order of appearance; `data-zephium-link` indexes this list. */
  links: string[];
  /** `http(s):` images in the message, shown or not. */
  remoteImages: number;
  /** Of those, how many are shown from the fetched `remoteImages` map. */
  remoteLoaded: number;
  /** `https:` image addresses the backend may be asked to fetch, in order, without repeats. */
  remoteUrls: string[];
  /** Images removed because they could not be shown safely. */
  droppedImages: number;
};

/** Longest entity body (between `&` and `;`) that is decoded. */
const MAX_ENTITY_BODY = 9;

/**
 * Decodes the entities of an attribute value. The rules are those of
 * `decode_entities` in `crates/zephium-mail/src/remote_images.rs`, which finds
 * the same image addresses on the backend: a `;` is required, the body is at
 * most nine characters, names are not case sensitive, and zero, a surrogate or
 * a number past U+10FFFF becomes U+FFFD. The fixture
 * `tests/image-url-parity.json` keeps the two in step.
 */
function decodeEntities(value: string): string {
  return value.replace(/&(#x[0-9a-f]+|#[0-9]+|[a-z][a-z0-9]*);/giu, (whole, body: string) => {
    if (body.length > MAX_ENTITY_BODY) return whole;
    if (body.startsWith("#")) {
      const code =
        body[1] === "x" || body[1] === "X"
          ? parseInt(body.slice(2), 16)
          : parseInt(body.slice(1), 10);
      if (!Number.isFinite(code) || code <= 0 || code > 0x10ffff) return "\ufffd";
      if (code >= 0xd800 && code <= 0xdfff) return "\ufffd";
      return String.fromCodePoint(code);
    }
    return NAMED_ENTITIES[body.toLowerCase()] ?? whole;
  });
}

const escapeText = (value: string) =>
  value
    .replace(/&(?!(?:#x?[0-9a-f]+|[a-z][a-z0-9]*);)/giu, "&amp;")
    .replace(/</gu, "&lt;")
    .replace(/>/gu, "&gt;");

const escapeAttribute = (value: string) =>
  value
    .replace(/&/gu, "&amp;")
    .replace(/"/gu, "&quot;")
    .replace(/</gu, "&lt;")
    .replace(/>/gu, "&gt;");

/** C0 and C1 controls, space, and the invisible characters browsers skip in a scheme. */
const invisible = (code: number) =>
  code <= 0x20 ||
  (code >= 0x7f && code <= 0x9f) ||
  (code >= 0x200b && code <= 0x200f) ||
  code === 0x2028 ||
  code === 0x2029 ||
  code === 0xfeff;

const hasControl = (value: string) => {
  for (let at = 0; at < value.length; at += 1) {
    const code = value.charCodeAt(at);
    if (code <= 0x1f || code === 0x7f) return true;
  }
  return false;
};

/** The scheme of a URL the way a browser would read it, or `null` when relative. */
function schemeOf(raw: string): string | null {
  // Browsers drop tabs, newlines and leading C0 controls or spaces before parsing.
  const decoded = decodeEntities(raw);
  let cleaned = "";
  for (let at = 0; at < decoded.length; at += 1) {
    if (!invisible(decoded.charCodeAt(at))) cleaned += decoded[at];
  }
  const match = /^([a-z][a-z0-9+.-]*):/iu.exec(cleaned);
  return match ? match[1]!.toLowerCase() : null;
}

/** Removes the invisible characters browsers skip in a scheme; a plain space stays. */
function stripInvisible(value: string): string {
  let cleaned = "";
  for (let at = 0; at < value.length; at += 1) {
    const code = value.charCodeAt(at);
    if (code === 0x20 || !invisible(code)) cleaned += value[at];
  }
  return cleaned;
}

/**
 * A link target that can be shown and opened as a tab, or `null`. The result
 * is the cleaned form that was checked: no zero-width or other invisible
 * characters remain in it.
 */
export function safeLinkTarget(raw: string): string | null {
  const decoded = decodeEntities(raw).trim();
  if (hasControl(decoded)) return null;
  const cleaned = stripInvisible(decoded);
  const scheme = schemeOf(cleaned);
  if (scheme !== "http" && scheme !== "https" && scheme !== "mailto") return null;
  if (scheme !== "mailto" && /\s/u.test(cleaned)) return null;
  return cleaned.length > 2048 ? null : cleaned;
}

/** Decoded size of a base64 payload without decoding it. */
function base64Bytes(payload: string): number {
  const clean = payload.replace(/\s+/gu, "");
  const padding = clean.endsWith("==") ? 2 : clean.endsWith("=") ? 1 : 0;
  return Math.floor((clean.length * 3) / 4) - padding;
}

const IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

/** A `data:` URL for a raster image within the size cap, or `null`. SVG is refused. */
function safeImageDataUrl(raw: string): string | null {
  const value = decodeEntities(raw).trim();
  const match = /^data:([a-z]+\/[a-z0-9.+-]+);base64,([a-z0-9+/=\s]*)$/iu.exec(value);
  if (!match) return null;
  const type = match[1]!.toLowerCase();
  if (!IMAGE_TYPES.has(type)) return null;
  if (base64Bytes(match[2]!) > MAX_INLINE_IMAGE_BYTES) return null;
  return `data:${type};base64,${match[2]!.replace(/\s+/gu, "")}`;
}

/** Builds the `data:` URL for an inline attachment, or `null` when unsafe or too big. */
export function inlineImageUrl(contentType: string, base64: string): string | null {
  const type = contentType.split(";")[0]!.trim().toLowerCase();
  if (!IMAGE_TYPES.has(type)) return null;
  if (!/^[a-z0-9+/=\s]*$/iu.test(base64)) return null;
  if (base64Bytes(base64) > MAX_INLINE_IMAGE_BYTES) return null;
  return `data:${type};base64,${base64.replace(/\s+/gu, "")}`;
}

/** Filters an inline `style` attribute to listed, resource-free properties. */
export function sanitizeStyle(raw: string): string {
  const text = decodeEntities(raw).replace(/\/\*[\s\S]*?\*\//gu, "");
  const kept: string[] = [];
  for (const declaration of text.split(";")) {
    const at = declaration.indexOf(":");
    if (at < 0) continue;
    const property = declaration.slice(0, at).trim().toLowerCase();
    const value = declaration.slice(at + 1).trim();
    if (!STYLE_PROPERTIES.has(property) || !value || value.length > 200) continue;
    // Anything that can fetch, script or escape the declaration is refused whole.
    if (
      hasControl(value) ||
      /[\\@<>{}]|url\s*\(|image|expression|javascript|behavior|binding|attr\s*\(|var\s*\(|\/\*/iu.test(
        value,
      )
    ) {
      continue;
    }
    if (property === "display" && !SAFE_DISPLAY.has(value.toLowerCase())) continue;
    kept.push(`${property}: ${value}`);
  }
  return kept.join("; ");
}

type Attribute = { name: string; value: string };

type Tag = {
  name: string;
  closing: boolean;
  selfClosing: boolean;
  attributes: Attribute[];
  end: number;
};

/** Reads one tag starting at `<`; `null` when it is not a tag or never ends. */
function readTag(source: string, start: number): Tag | null {
  const head = /^<(\/?)([a-z][^\s/>]*)/iu.exec(source.slice(start, start + 64));
  if (!head) return null;
  let at = start + head[0].length;
  const attributes: Attribute[] = [];
  let selfClosing = false;
  while (at < source.length) {
    const ch = source[at]!;
    if (ch === ">") {
      return {
        name: head[2]!.toLowerCase(),
        closing: head[1] === "/",
        selfClosing,
        attributes,
        end: at + 1,
      };
    }
    if (/\s/u.test(ch)) {
      at += 1;
      continue;
    }
    if (ch === "/") {
      selfClosing = true;
      at += 1;
      continue;
    }
    selfClosing = false;
    let nameEnd = at;
    while (nameEnd < source.length && !/[\s/>=]/u.test(source[nameEnd]!)) nameEnd += 1;
    if (nameEnd === at) {
      // A stray `=` or quote starts a name in the HTML parser.
      nameEnd = at + 1;
    }
    const name = source.slice(at, nameEnd).toLowerCase();
    at = nameEnd;
    while (at < source.length && /\s/u.test(source[at]!)) at += 1;
    let value = "";
    if (source[at] === "=") {
      at += 1;
      while (at < source.length && /\s/u.test(source[at]!)) at += 1;
      const quote = source[at];
      if (quote === '"' || quote === "'") {
        const close = source.indexOf(quote, at + 1);
        if (close < 0) return null;
        value = source.slice(at + 1, close);
        at = close + 1;
      } else {
        const begin = at;
        while (at < source.length && !/[\s>]/u.test(source[at]!)) at += 1;
        value = source.slice(begin, at);
      }
    }
    attributes.push({ name, value });
  }
  return null;
}

/** The index just after the close tag of a raw-text element, or the end. */
function skipRawText(source: string, from: number, name: string): number {
  const closer = new RegExp(`</${name}(?=[\\s/>])`, "iu");
  const found = closer.exec(source.slice(from));
  if (!found) return source.length;
  const close = readTag(source, from + found.index);
  return close ? close.end : source.length;
}

export function sanitizeMailHtml(input: string, options: SanitizeOptions = {}): SanitizedMail {
  const source = input.length > MAX_HTML_CHARS ? input.slice(0, MAX_HTML_CHARS) : input;
  const inline = options.inline ?? new Map<string, string>();
  const out: string[] = [];
  const links: string[] = [];
  const open: string[] = [];
  const remoteUrls: string[] = [];
  const remoteSeen = new Set<string>();
  let remoteImages = 0;
  let remoteLoaded = 0;
  let droppedImages = 0;
  let skipDepth = 0;
  let at = 0;

  const writeImage = (attributes: Attribute[]) => {
    const get = (name: string) => attributes.find((a) => a.name === name)?.value;
    const src = get("src");
    const parts: string[] = [];
    for (const attribute of attributes) {
      if (!TAG_ATTRIBUTES.img!.has(attribute.name) && !GLOBAL_ATTRIBUTES.has(attribute.name))
        continue;
      parts.push(`${attribute.name}="${escapeAttribute(decodeEntities(attribute.value))}"`);
    }
    const style = get("style");
    const safeStyle = style === undefined ? "" : sanitizeStyle(style);
    if (safeStyle) parts.push(`style="${escapeAttribute(safeStyle)}"`);
    const scheme = src === undefined ? null : schemeOf(src);
    let shown: string | null = null;
    let kind: "inline" | "remote" | "blocked" = "blocked";
    if (src !== undefined && scheme === "cid") {
      const id = decodeEntities(src).trim().slice(4).replace(/^<|>$/gu, "");
      shown = inline.get(id) ?? null;
      kind = "inline";
    } else if (src !== undefined && scheme === "data") {
      shown = safeImageDataUrl(src);
      kind = "inline";
    } else if (src !== undefined && (scheme === "http" || scheme === "https")) {
      remoteImages += 1;
      kind = "remote";
      const target = safeLinkTarget(src);
      // Only an `https:` address can be fetched; the answer is re-checked as a raster `data:` URL.
      if (target !== null && schemeOf(target) === "https") {
        // At most what the backend accepts; a Set keeps the check constant time.
        if (!remoteSeen.has(target) && remoteUrls.length < MAX_REMOTE_IMAGES) {
          remoteSeen.add(target);
          remoteUrls.push(target);
        }
        const fetched = options.remoteImages?.get(target);
        shown = fetched === undefined ? null : safeImageDataUrl(fetched);
        if (shown !== null) remoteLoaded += 1;
      }
    }
    if (shown === null && kind !== "remote") droppedImages += 1;
    if (shown !== null) parts.push(`src="${escapeAttribute(shown)}"`);
    else parts.push('data-zephium-blocked="1"');
    out.push(`<img ${parts.join(" ")}>`);
  };

  const writeAnchor = (attributes: Attribute[]) => {
    const href = attributes.find((a) => a.name === "href")?.value;
    const target = href === undefined || links.length >= MAX_LINKS ? null : safeLinkTarget(href);
    const parts: string[] = [];
    const title = attributes.find((a) => a.name === "title")?.value;
    // The real target is the tooltip; the author's own title could disguise it.
    if (target !== null) {
      links.push(target);
      parts.push(`data-zephium-link="${links.length - 1}"`);
      parts.push(`title="${escapeAttribute(target)}"`);
    } else if (title !== undefined) {
      parts.push(`title="${escapeAttribute(decodeEntities(title))}"`);
    }
    const style = attributes.find((a) => a.name === "style")?.value;
    const safeStyle = style === undefined ? "" : sanitizeStyle(style);
    if (safeStyle) parts.push(`style="${escapeAttribute(safeStyle)}"`);
    out.push(`<a${parts.length ? " " : ""}${parts.join(" ")}>`);
  };

  const writeTag = (tag: Tag) => {
    const { name, attributes } = tag;
    if (name === "img") return writeImage(attributes);
    if (name === "a") return writeAnchor(attributes);
    const allowed = TAG_ATTRIBUTES[name];
    const parts: string[] = [];
    for (const attribute of attributes) {
      if (attribute.name === "style") {
        const style = sanitizeStyle(attribute.value);
        if (style) parts.push(`style="${escapeAttribute(style)}"`);
        continue;
      }
      if (!GLOBAL_ATTRIBUTES.has(attribute.name) && !allowed?.has(attribute.name)) continue;
      parts.push(`${attribute.name}="${escapeAttribute(decodeEntities(attribute.value))}"`);
    }
    out.push(`<${name}${parts.length ? " " : ""}${parts.join(" ")}>`);
  };

  while (at < source.length) {
    const lt = source.indexOf("<", at);
    if (lt < 0) {
      if (skipDepth === 0) out.push(escapeText(source.slice(at)));
      break;
    }
    if (lt > at && skipDepth === 0) out.push(escapeText(source.slice(at, lt)));
    // Comments, doctype, CDATA, processing instructions and conditional comments.
    if (source.startsWith("<!--", lt)) {
      const close = source.indexOf("-->", lt + 4);
      at = close < 0 ? source.length : close + 3;
      continue;
    }
    if (source[lt + 1] === "!" || source[lt + 1] === "?") {
      const close = source.indexOf(">", lt + 2);
      at = close < 0 ? source.length : close + 1;
      continue;
    }
    const tag = readTag(source, lt);
    if (!tag) {
      // Not a tag: a literal `<`, unless the tag never ends, which drops the rest.
      if (/^<\/?[a-z]/iu.test(source.slice(lt, lt + 3))) {
        at = source.length;
        continue;
      }
      if (skipDepth === 0) out.push("&lt;");
      at = lt + 1;
      continue;
    }
    at = tag.end;
    const { name } = tag;
    if (DROP_WITH_CONTENT.has(name)) {
      if (name === "embed" || name === "frame") continue; // no content, no close tag
      if (tag.closing) {
        if (skipDepth > 0) skipDepth -= 1;
      } else if (RAW_TEXT.has(name)) {
        at = skipRawText(source, at, name);
      } else if (!tag.selfClosing) {
        skipDepth += 1;
      }
      continue;
    }
    if (skipDepth > 0) continue;
    if (!ALLOWED_TAGS.has(name)) continue; // unknown or form: unwrapped, children stay
    if (tag.closing) {
      const index = open.lastIndexOf(name);
      if (index < 0) continue;
      // Close anything left open inside it so the markup stays balanced.
      while (open.length > index) out.push(`</${open.pop()}>`);
      continue;
    }
    if (VOID_TAGS.has(name)) {
      writeTag(tag);
      continue;
    }
    if (open.length >= MAX_DEPTH) continue;
    writeTag(tag);
    open.push(name);
  }
  while (open.length) out.push(`</${open.pop()}>`);
  return { html: out.join(""), links, remoteImages, remoteLoaded, remoteUrls, droppedImages };
}

/** The Content-Security-Policy of the reading pane's document. */
export function mailFrameCsp(): string {
  return "default-src 'none'; img-src data:; style-src 'unsafe-inline'";
}

const FRAME_STYLE =
  "html{color-scheme:light}body{margin:0;padding:12px;background:#fff;color:#1b1b1f;" +
  "font:14px/1.5 system-ui,sans-serif;overflow-wrap:anywhere}img{max-width:100%;height:auto}" +
  "table{max-width:100%}a[data-zephium-link]{color:#1a56c4;text-decoration:underline;cursor:default}" +
  "img[data-zephium-blocked]{display:inline-block;min-width:16px;min-height:16px;background:#ececf0}" +
  "blockquote{margin:8px 0;padding-left:12px;border-left:3px solid #c9c9d1}pre{white-space:pre-wrap}";

/** A complete `srcdoc` for a sandboxed frame around already sanitised markup. */
export function mailFrameDocument(sanitizedBody: string): string {
  return (
    `<!doctype html><html><head><meta charset="utf-8">` +
    `<meta http-equiv="Content-Security-Policy" content="${mailFrameCsp()}">` +
    `<style>${FRAME_STYLE}</style></head><body>${sanitizedBody}</body></html>`
  );
}

const URL_PATTERN = /\b(?:https?:\/\/|mailto:)[^\s<>"')\]]+/giu;

type TextSegment = { text: string; url?: undefined } | { text: string; url: string };

/** Splits plain text into text and link segments; the text is never markup. */
export function linkify(text: string): TextSegment[] {
  const segments: TextSegment[] = [];
  let last = 0;
  for (const match of text.matchAll(URL_PATTERN)) {
    const trimmed = match[0].replace(/[.,;:!?]+$/u, "");
    const target = safeLinkTarget(trimmed);
    if (!target) continue;
    if (match.index > last) segments.push({ text: text.slice(last, match.index) });
    segments.push({ text: trimmed, url: target });
    last = match.index + trimmed.length;
  }
  if (last < text.length) segments.push({ text: text.slice(last) });
  return segments;
}
