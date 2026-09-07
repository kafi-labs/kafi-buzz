import { Outlet, createRootRoute } from "@tanstack/react-router";

import { BunkerSignerProvider } from "@/shared/context/BunkerSignerContext";

export const Route = createRootRoute({
  component: RootLayout,
});

function RootLayout() {
  return (
    <BunkerSignerProvider>
      <div className="flex min-h-dvh flex-col">
        <main className="flex flex-1 flex-col">
          <Outlet />
        </main>
      </div>
    </BunkerSignerProvider>
  );
}
