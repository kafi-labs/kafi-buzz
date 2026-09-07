/**
 * One shared NIP-46 bunker connection for the whole app — mounted once at
 * the router root so `InvitePage` (durable relay membership) and
 * `ChatClientContext` (chat signing) read the SAME connection rather than
 * each independently instantiating one. That sharing is a correctness
 * requirement, not a nicety: a `BunkerSigner` holds a live relay-pool
 * subscription, so two instances from one bunker pointer would open two
 * concurrent RPC channels to the same bunker for one logical identity.
 */
import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";

import {
  type BunkerConnectionState,
  type Nip46Signer,
  clearPersistedBunker,
  connectBunker,
  connectToBunkerPointer,
  loadPersistedBunkerPointer,
} from "@/shared/lib/bunker-signer";

interface BunkerSignerContextValue {
  state: BunkerConnectionState;
  /** Non-null exactly when `state === "connected"`. */
  signer: Nip46Signer | null;
  /** Connect to a fresh `bunker://` URI. Rejects on malformed input without touching connection state. */
  connect: (bunkerUri: string) => Promise<void>;
  /** Local teardown and forget the persisted pointer. Does not revoke the session at the bunker. */
  disconnect: () => void;
}

const BunkerSignerContext = createContext<BunkerSignerContextValue | null>(
  null,
);

export function BunkerSignerProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<BunkerConnectionState>("disconnected");
  const [signer, setSigner] = useState<Nip46Signer | null>(null);
  // Fences async connection attempts by generation, matching the
  // generation-fence rule this repo already applies to relay-side
  // async probes: a stale attempt's eventual settlement must not overwrite
  // a newer attempt's state.
  const generationRef = useRef(0);
  const autoReconnectAttempted = useRef(false);

  const applyIfCurrent = useCallback(
    (generation: number, apply: () => void) => {
      if (generation === generationRef.current) apply();
    },
    [],
  );

  /** Bump the generation and reset for a fresh attempt; returns the fenced state-change callback to hand to the connector. */
  const beginAttempt = useCallback(() => {
    const generation = ++generationRef.current;
    setSigner(null);
    const onStateChange = (next: BunkerConnectionState) =>
      applyIfCurrent(generation, () => setState(next));
    return { generation, onStateChange };
  }, [applyIfCurrent]);

  useEffect(() => {
    // Guards against React StrictMode's dev-only double-invoke opening two
    // live relay-pool subscriptions for one persisted pointer — the
    // generation fence alone stops a stale result from being *applied*, but
    // the underlying connection would still have been *opened* twice.
    if (autoReconnectAttempted.current) return;
    autoReconnectAttempted.current = true;
    const persisted = loadPersistedBunkerPointer();
    if (!persisted) return;
    const { generation, onStateChange } = beginAttempt();
    connectToBunkerPointer(persisted, onStateChange)
      .then((connected) => {
        applyIfCurrent(generation, () => setSigner(connected));
      })
      .catch(() => {
        // Terminal failure is already reflected via onStateChange.
      });
  }, [beginAttempt, applyIfCurrent]);

  const connect = useCallback(
    (bunkerUri: string) => {
      const { generation, onStateChange } = beginAttempt();
      return connectBunker(bunkerUri, onStateChange).then((connected) => {
        applyIfCurrent(generation, () => setSigner(connected));
      });
      // Malformed input / terminal failures reject naturally here — the
      // caller (a form) awaits this and shows its own message. Anything
      // that got far enough to call onStateChange already updated `state`.
    },
    [beginAttempt, applyIfCurrent],
  );

  const disconnect = useCallback(() => {
    generationRef.current++;
    setSigner((current) => {
      void current?.disconnect().catch(() => {});
      return null;
    });
    clearPersistedBunker();
    setState("disconnected");
  }, []);

  return (
    <BunkerSignerContext.Provider
      value={{ state, signer, connect, disconnect }}
    >
      {children}
    </BunkerSignerContext.Provider>
  );
}

export function useBunkerSigner(): BunkerSignerContextValue {
  const ctx = useContext(BunkerSignerContext);
  if (!ctx) {
    throw new Error(
      "useBunkerSigner must be used within a BunkerSignerProvider",
    );
  }
  return ctx;
}
