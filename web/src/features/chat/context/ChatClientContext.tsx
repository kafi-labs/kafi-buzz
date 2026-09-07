/**
 * Supplies one `BuzzClient` (and its resolved identity) across the chat UI
 * tree for the lifetime of the provider. Scoped to a single channel view for
 * the Phase 2 MVP — hoisting it above channel switches, so the socket
 * survives navigation, is Phase 3's job once there's a sidebar to switch from.
 */
import {
  type ReactNode,
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { useBunkerSigner } from "@/shared/context/BunkerSignerContext";
import {
  SignerRequestError,
  type SignerFailureKind,
} from "@/shared/lib/bunker-signer";
import { BuzzClient } from "@/shared/lib/buzz-client";
import {
  EphemeralSigner,
  LocalKeySigner,
  Nip07Signer,
  type Signer,
  hasNip07Provider,
} from "@/shared/lib/nostr-signer";
import type { ConnectionState } from "@/shared/lib/nostr-types";

/**
 * NIP-07 extension when present (sovereign identity), otherwise a
 * `localStorage`-persisted key so a reload keeps the same identity without
 * requiring an extension. `EphemeralSigner` is deliberately not the
 * composer's default — a page-lifetime key would make every reload look like
 * a different author. A connected NIP-46 bunker takes priority over all of
 * these — see `useBunkerSigner` below — this is only the fallback.
 */
function resolveFallbackSigner(): Signer {
  if (hasNip07Provider()) {
    return new Nip07Signer();
  }
  try {
    return new LocalKeySigner();
  } catch {
    // localStorage unavailable (private browsing, storage disabled) — fall
    // back to a read-mostly identity rather than throwing on mount.
    return new EphemeralSigner();
  }
}

interface ChatClientContextValue {
  client: BuzzClient;
  connectionState: ConnectionState;
  connectionDetail: string | undefined;
  pubkey: string | null;
  /**
   * Why the last `getPublicKey()` call failed, when it was a classifiable
   * signer failure — `null` otherwise (including "no failure" and "failed
   * for some other reason"). Kept separate from `connectionState`/
   * `connectionDetail`: a signer can be connected while a specific pubkey
   * read still fails, and that must not collapse into the relay's own
   * connection banner.
   */
  pubkeyError: SignerFailureKind | null;
}

const ChatClientContext = createContext<ChatClientContextValue | null>(null);

export function ChatClientProvider({ children }: { children: ReactNode }) {
  // "bunker connected" (can I sign?) and "relay member" (may I post?,
  // reflected below via connectionState) are independent axes — a bunker
  // disconnecting must swap the client to the fallback signer rather than
  // silently keep using a signer that can no longer be reached, and a fresh
  // bunker connection must swap the client TO it rather than leave chat
  // authenticated as whatever identity happened to resolve first.
  const { state: bunkerState, signer: bunkerSigner } = useBunkerSigner();
  const fallbackSignerRef = useRef<Signer | null>(null);
  if (!fallbackSignerRef.current) {
    fallbackSignerRef.current = resolveFallbackSigner();
  }
  const activeSigner: Signer =
    bunkerState === "connected" && bunkerSigner
      ? bunkerSigner
      : fallbackSignerRef.current;

  const [client, setClient] = useState<BuzzClient>(
    () => new BuzzClient({ signer: activeSigner }),
  );
  // Which signer identity `client` was actually built with — only
  // reconstruct when this changes, not on every render.
  const clientSignerRef = useRef<Signer>(activeSigner);

  const [connectionState, setConnectionState] = useState<ConnectionState>(
    client.getConnectionState(),
  );
  const [connectionDetail, setConnectionDetail] = useState<string | undefined>(
    undefined,
  );
  const [pubkey, setPubkey] = useState<string | null>(null);
  const [pubkeyError, setPubkeyError] = useState<SignerFailureKind | null>(
    null,
  );

  useEffect(() => {
    if (clientSignerRef.current === activeSigner) return;
    clientSignerRef.current = activeSigner;
    setClient(new BuzzClient({ signer: activeSigner }));
  }, [activeSigner]);

  useEffect(() => {
    const unsubscribe = client.onConnectionChange((state, detail) => {
      setConnectionState(state);
      setConnectionDetail(detail);
    });
    void client.connect().catch(() => {
      // Failure is already reflected via the connectionState listener above.
    });
    let cancelled = false;
    void client
      .getPublicKey()
      .then((pk) => {
        if (cancelled) return;
        setPubkey(pk);
        setPubkeyError(null);
      })
      .catch((error) => {
        if (cancelled) return;
        setPubkey(null);
        setPubkeyError(error instanceof SignerRequestError ? error.kind : null);
      });

    return () => {
      cancelled = true;
      unsubscribe();
      client.disconnect();
    };
  }, [client]);

  const value = useMemo<ChatClientContextValue>(
    () => ({ client, connectionState, connectionDetail, pubkey, pubkeyError }),
    [client, connectionState, connectionDetail, pubkey, pubkeyError],
  );

  return (
    <ChatClientContext.Provider value={value}>
      {children}
    </ChatClientContext.Provider>
  );
}

export function useChatClient(): ChatClientContextValue {
  const ctx = useContext(ChatClientContext);
  if (!ctx) {
    throw new Error("useChatClient must be used within a ChatClientProvider");
  }
  return ctx;
}
