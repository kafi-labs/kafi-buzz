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
 * a different author.
 */
function resolveDefaultSigner(): Signer {
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
}

const ChatClientContext = createContext<ChatClientContextValue | null>(null);

export function ChatClientProvider({ children }: { children: ReactNode }) {
  const clientRef = useRef<BuzzClient | null>(null);
  if (!clientRef.current) {
    clientRef.current = new BuzzClient({ signer: resolveDefaultSigner() });
  }
  const client = clientRef.current;

  const [connectionState, setConnectionState] = useState<ConnectionState>(
    client.getConnectionState(),
  );
  const [connectionDetail, setConnectionDetail] = useState<string | undefined>(
    undefined,
  );
  const [pubkey, setPubkey] = useState<string | null>(null);

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
        if (!cancelled) setPubkey(pk);
      })
      .catch(() => {
        if (!cancelled) setPubkey(null);
      });

    return () => {
      cancelled = true;
      unsubscribe();
      client.disconnect();
    };
  }, [client]);

  const value = useMemo<ChatClientContextValue>(
    () => ({ client, connectionState, connectionDetail, pubkey }),
    [client, connectionState, connectionDetail, pubkey],
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
