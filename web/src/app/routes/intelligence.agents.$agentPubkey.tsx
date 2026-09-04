import { createFileRoute } from "@tanstack/react-router";
import { AgentDetailPage } from "@/features/intelligence/ui/AgentDetailPage";

export const Route = createFileRoute("/intelligence/agents/$agentPubkey")({
  component: AgentDetailRoute,
});

function AgentDetailRoute() {
  const { agentPubkey } = Route.useParams();
  return <AgentDetailPage agentPubkey={agentPubkey} />;
}
