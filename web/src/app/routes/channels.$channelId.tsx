import { createFileRoute } from "@tanstack/react-router";
import { ChannelPage } from "@/features/chat/ui/ChannelPage";

export const Route = createFileRoute("/channels/$channelId")({
  component: ChannelPageRoute,
});

function ChannelPageRoute() {
  const { channelId } = Route.useParams();
  return <ChannelPage channelId={channelId} />;
}
