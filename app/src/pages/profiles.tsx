// Settings → Profiles: the AI profiles panel, as it was under "AI profiles".
import ProfilesPanel from "../ProfilesPanel";
import { Page, Sec, useSheet } from "./kit";

export function ProfilesPage() {
  const { repo, onReport } = useSheet();
  return <Page page="profiles">
    <Sec id="profiles">
      <ProfilesPanel key={repo || "none"} repo={repo} onReport={onReport} />
    </Sec>
  </Page>;
}
