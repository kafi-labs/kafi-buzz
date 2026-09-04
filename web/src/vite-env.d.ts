/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** WebSocket URL of the relay. Defaults to the serving origin. */
  readonly VITE_RELAY_URL?: string;
  /**
   * Space/comma-separated hex pubkeys of workspace owners the Intelligence
   * console trusts as authors of persona and managed-agent events.
   */
  readonly VITE_CONSOLE_OWNER_PUBKEYS?: string;
  /**
   * Space/comma-separated hex pubkeys of agent runners the console trusts as
   * authors of runtime-status and gateway-catalog events.
   */
  readonly VITE_CONSOLE_RUNNER_PUBKEYS?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
