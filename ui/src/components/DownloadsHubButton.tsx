import { useDownloadsStore } from "../stores/downloadsStore";
import { IconDownload } from "./Icons";

/** Toolbar-sized button that opens the Downloads hub. Sits where the old
 * shuffle-favourites button lived in the desktop grid/suggestion headers. */
export default function DownloadsHubButton() {
  const openHub = useDownloadsStore((s) => s.openHub);
  return (
    <button className="filter-dropdown-btn" onClick={openHub} title="Downloads">
      <IconDownload size={14} />
    </button>
  );
}
