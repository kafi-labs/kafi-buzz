import { createFileRoute } from "@tanstack/react-router";

import { IntelligencePage } from "@/features/intelligence/ui/IntelligencePage";

export const Route = createFileRoute("/intelligence")({
  component: IntelligencePage,
});
