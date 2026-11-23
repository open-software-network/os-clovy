import type { SidebarView } from "../../components/sidebar/Sidebar";

// A navigation snapshot: everything needed to re-render a view exactly as the
// user left it. Each open tab owns one of these. Fields are gated by `view`
// (a note id is meaningless on the Routines view), which keeps `navEquals`
// stable — only the fields that matter for a view participate in equality.
export type TabNav = {
  view: SidebarView;
  // view === "meetings"
  noteId?: string;
  originFolderId?: string;
  originAllNotes?: boolean;
  // view === "folders"
  folderId?: string;
};

export type Tab = {
  id: string;
  nav: TabNav;
};

type LegacyChatView = "home" | "agent" | "agent-sessions";
type LegacyChatTabNav = {
  [View in LegacyChatView]: { view: View; agentSessionId?: string };
}[LegacyChatView];

export function defaultNav(): TabNav {
  return { view: "today" };
}

export function normalizeTabNav(nav: TabNav | LegacyChatTabNav): TabNav {
  if (nav.view === "home" || nav.view === "agent" || nav.view === "agent-sessions") {
    return defaultNav();
  }
  return nav;
}

export function makeTabId(): string {
  // crypto.randomUUID is available in the WKWebView runtime; the fallback keeps
  // unit tests (jsdom without crypto) from throwing.
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `tab-${Math.random().toString(36).slice(2)}`;
}

// Apply a drag-reorder of the on-strip tabs. `orderedVisibleIds` is the strip's
// visible tabs in their new left-to-right order; they become the leading prefix
// of the full array (so re-layout reproduces the strip exactly, even when the
// active tab had been pinned onto the strip from overflow), and the remaining
// tabs follow in their existing relative order.
export function reorderTabs(tabs: Tab[], orderedVisibleIds: string[]): Tab[] {
  const byId = new Map(tabs.map((tab) => [tab.id, tab]));
  const ordered = [...new Set(orderedVisibleIds)].filter((id) => byId.has(id));
  const orderedSet = new Set(ordered);
  const orderedTabs = ordered
    .map((id) => byId.get(id))
    .filter((tab): tab is Tab => tab !== undefined);
  const next = [...orderedTabs, ...tabs.filter((tab) => !orderedSet.has(tab.id))];
  if (next.every((tab, index) => tab === tabs[index])) return tabs;
  return next;
}

// A data partition switch can leave inactive tabs pointing at notes from the
// previous partition. Replace only those snapshots with the neutral notes list;
// callers may omit an active recording note from `noteIds` until the take ends.
export function invalidateNoteTabs(tabs: Tab[], noteIds: ReadonlySet<string>): Tab[] {
  let changed = false;
  const next = tabs.map<Tab>((tab) => {
    if (tab.nav.view !== "meetings" || !tab.nav.noteId || !noteIds.has(tab.nav.noteId)) {
      return tab;
    }
    changed = true;
    return { ...tab, nav: { view: "notes" } };
  });
  return changed ? next : tabs;
}

// Equality scoped to the fields that matter for the given view, so the capture
// effect doesn't churn the active tab when irrelevant live state shifts.
export function navEquals(a: TabNav, b: TabNav): boolean {
  if (a.view !== b.view) return false;
  switch (a.view) {
    case "meetings":
      return (
        a.noteId === b.noteId &&
        a.originFolderId === b.originFolderId &&
        !!a.originAllNotes === !!b.originAllNotes
      );
    case "folders":
      return a.folderId === b.folderId;
    default:
      return true;
  }
}
