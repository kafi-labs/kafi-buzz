import { createFileRoute } from "@tanstack/react-router";
import { IntelligenceOverviewPage } from "@/features/intelligence/ui/IntelligenceOverviewPage";

export const Route = createFileRoute("/intelligence")({
  component: IntelligenceOverviewPage,
});
