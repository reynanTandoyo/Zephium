export { MailStore, ALL_INBOXES, PAGE_SIZE, orderFolders } from "./mail.svelte";
export type { Load, MailNotice, MailSelection, OpenMessage } from "./mail.svelte";
export { AccountSetup } from "./setup.svelte";
export type { EndpointForm } from "./setup.svelte";
export { ComposeSession } from "./compose.svelte";
export type { ComposeAttachment, ComposeProblem } from "./compose.svelte";
export type { ComposeMode } from "./compose";
export { MAX_ATTACHMENT_BYTES } from "./compose";
export type { MailFailure } from "./call";
export { buildThreads, rowKey } from "./thread";
export type { MailRow, MailThread } from "./thread";
export { displayName, formatAddress, formatAddresses, parseAddresses } from "./address";
export {
  inlineImageUrl,
  linkify,
  mailFrameDocument,
  safeLinkTarget,
  sanitizeMailHtml,
} from "./sanitize";
export type { SanitizedMail, TextSegment } from "./sanitize";
