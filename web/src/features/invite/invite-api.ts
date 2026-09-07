import { makeNip98AuthHeader } from "@/shared/lib/nip98";
import { relayHttpBaseUrl } from "@/shared/lib/relay-url";

const INVITE_REQUEST_TIMEOUT_MS = 15_000;
const KIND_CHANNEL_METADATA = 39000;
/** NIP-29 discovery tags the relay emits for a channel's visibility. */
const NIP29_OPEN_VISIBILITY_TAG = "public";

export type BrowserInviteClaim = {
  status: "joined" | "already_member";
  communityId: string;
  host: string;
  role: string;
};

export async function claimInviteInBrowser(
  code: string,
  policyReceipt?: string,
): Promise<BrowserInviteClaim> {
  const url = `${relayHttpBaseUrl().replace(/\/+$/, "")}/api/invites/claim`;
  const body = JSON.stringify({
    code,
    policy_receipt: policyReceipt,
  });
  const authorization = await makeNip98AuthHeader(url, "POST", {
    body,
    requireNip07: true,
  });
  const response = await fetch(url, {
    method: "POST",
    headers: {
      Authorization: authorization,
      "Content-Type": "application/json",
    },
    body,
    signal: AbortSignal.timeout(INVITE_REQUEST_TIMEOUT_MS),
  });
  const json = (await response.json().catch(() => ({}))) as Record<
    string,
    unknown
  >;
  if (!response.ok) {
    const message =
      typeof json.error === "string" ? json.error : `HTTP ${response.status}`;
    throw new Error(message);
  }

  return {
    status: json.status as BrowserInviteClaim["status"],
    communityId: String(json.community_id),
    host: String(json.host),
    role: String(json.role),
  };
}

type NostrEventTag = string[];
type NostrEventLike = { tags?: NostrEventTag[] };

export function isOpenChannelMetadata(event: NostrEventLike): boolean {
  return (event.tags ?? []).some(
    (tag) => tag.length === 1 && tag[0] === NIP29_OPEN_VISIBILITY_TAG,
  );
}

export function channelIdFromMetadata(event: NostrEventLike): string | null {
  const dTag = (event.tags ?? []).find((tag) => tag[0] === "d");
  return dTag?.[1] ?? null;
}

/**
 * Find an `open`-visibility channel a newly claimed relay member can land
 * in without a second, separate channel-membership seating — invite-claim
 * only seats relay membership (never channel membership), and posting into
 * a `private` channel requires that second seating, so a private channel is
 * not a safe landing target here. Returns `null` when the community has no
 * open channel, so the caller can say so rather than navigating somewhere
 * that will silently fail to post.
 */
export async function findOpenChannelIdInBrowser(): Promise<string | null> {
  const url = `${relayHttpBaseUrl().replace(/\/+$/, "")}/query`;
  // The bridge takes a Nostr REQ-style array of filters, ORed together —
  // even a single filter must be wrapped, or it 400s ("invalid type: map,
  // expected a sequence").
  const body = JSON.stringify([{ kinds: [KIND_CHANNEL_METADATA] }]);
  const authorization = await makeNip98AuthHeader(url, "POST", {
    body,
    requireNip07: true,
  });
  const response = await fetch(url, {
    method: "POST",
    headers: {
      Authorization: authorization,
      "Content-Type": "application/json",
    },
    body,
    signal: AbortSignal.timeout(INVITE_REQUEST_TIMEOUT_MS),
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  const events = (await response.json().catch(() => [])) as NostrEventLike[];
  const openChannel = events.find(isOpenChannelMetadata);
  return openChannel ? channelIdFromMetadata(openChannel) : null;
}
