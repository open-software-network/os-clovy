import { describe, expect, it } from "vitest";
import {
  defaultNav,
  invalidateNoteTabs,
  navEquals,
  normalizeTabNav,
  reorderTabs,
  type Tab,
  type TabNav,
} from "../app/tabs/tabs";

describe("tab navigation snapshots", () => {
  it("a fresh tab opens Today", () => {
    expect(defaultNav()).toEqual({ view: "today" });
  });

  it("treats different views as different", () => {
    expect(navEquals({ view: "notes" }, { view: "routines" })).toBe(false);
  });

  it("compares only the fields a view actually shows", () => {
    // Section views carry no selection, so unrelated leftover fields are
    // ignored — this is what keeps the capture effect from churning the tab.
    const a: TabNav = { view: "routines", noteId: "n1" };
    const b: TabNav = { view: "routines", folderId: "f9" };
    expect(navEquals(a, b)).toBe(true);
  });

  it("distinguishes notes by id and by breadcrumb origin", () => {
    const base: TabNav = { view: "meetings", noteId: "n1" };
    expect(navEquals(base, { view: "meetings", noteId: "n2" })).toBe(false);
    expect(
      navEquals(base, {
        view: "meetings",
        noteId: "n1",
        originAllNotes: true,
      }),
    ).toBe(false);
    expect(
      navEquals(base, {
        view: "meetings",
        noteId: "n1",
        originFolderId: "f1",
      }),
    ).toBe(false);
    // Absent vs. explicitly-false origin is the same tab.
    expect(
      navEquals(base, {
        view: "meetings",
        noteId: "n1",
        originAllNotes: false,
      }),
    ).toBe(true);
  });

  it.each(["home", "agent", "agent-sessions"] as const)("normalizes legacy %s tabs", (view) => {
    expect(normalizeTabNav({ view, agentSessionId: "saved-session" })).toEqual({ view: "today" });
  });

  it("distinguishes projects by folder id", () => {
    expect(navEquals({ view: "folders" }, { view: "folders", folderId: "f1" })).toBe(false);
    expect(
      navEquals({ view: "folders", folderId: "f1" }, { view: "folders", folderId: "f1" }),
    ).toBe(true);
  });
});

describe("invalidateNoteTabs", () => {
  it("replaces only invalid note snapshots with the neutral notes list", () => {
    const tabs: Tab[] = [
      { id: "old", nav: { view: "meetings", noteId: "note-a", originFolderId: "folder-a" } },
      { id: "current", nav: { view: "meetings", noteId: "note-b" } },
      { id: "routines", nav: { view: "routines" } },
    ];

    expect(invalidateNoteTabs(tabs, new Set(["note-a"]))).toEqual([
      { id: "old", nav: { view: "notes" } },
      tabs[1],
      tabs[2],
    ]);
  });

  it("returns the original array when no note tab is invalid", () => {
    const tabs: Tab[] = [{ id: "current", nav: { view: "meetings", noteId: "note-b" } }];
    expect(invalidateNoteTabs(tabs, new Set(["note-a"]))).toBe(tabs);
  });
});

describe("reorderTabs", () => {
  const tab = (id: string): Tab => ({ id, nav: defaultNav() });
  const ids = (tabs: Tab[]) => tabs.map((t) => t.id);

  it("applies the new visible order", () => {
    const tabs = [tab("a"), tab("b"), tab("c")];
    expect(ids(reorderTabs(tabs, ["b", "c", "a"]))).toEqual(["b", "c", "a"]);
  });

  it("keeps overflowed (non-visible) tabs in their relative order after the strip", () => {
    // a, b, d are on the strip; c and e sit in the overflow popover. The dropped
    // strip order [d, b, a] leads, then c and e follow in their existing order.
    const tabs = [tab("a"), tab("b"), tab("c"), tab("d"), tab("e")];
    expect(ids(reorderTabs(tabs, ["d", "b", "a"]))).toEqual(["d", "b", "a", "c", "e"]);
  });

  it("preserves the dropped order when the active tab was pinned from overflow", () => {
    // Full order a..e with active e pinned onto the strip by layout: strip shows
    // [a, b, e]. Dragging e to the front must commit exactly that strip order —
    // slot-index reassignment used to scatter it to [e, a, c, d, b], which
    // re-layout then rendered as [e, a, c].
    const tabs = [tab("a"), tab("b"), tab("c"), tab("d"), tab("e")];
    expect(ids(reorderTabs(tabs, ["e", "a", "b"]))).toEqual(["e", "a", "b", "c", "d"]);
  });

  it("returns the same array when the order is unchanged", () => {
    const tabs = [tab("a"), tab("b")];
    expect(reorderTabs(tabs, ["a", "b"])).toBe(tabs);
  });

  it("ignores ids that no longer exist", () => {
    const tabs = [tab("a"), tab("b")];
    expect(ids(reorderTabs(tabs, ["b", "gone", "a"]))).toEqual(["b", "a"]);
  });
});
