/** Whether composer content is non-empty once whitespace is trimmed. */
export function isSendableContent(content: string): boolean {
  return content.trim().length > 0;
}

/**
 * Enter submits, Shift+Enter inserts a newline. Named and pure so both the
 * textarea's keydown handler and the Send button drive the exact same
 * decision — no divergence between the keyboard and pointer input paths.
 */
export function shouldSubmitOnKeyDown(event: {
  key: string;
  shiftKey: boolean;
}): boolean {
  return event.key === "Enter" && !event.shiftKey;
}
