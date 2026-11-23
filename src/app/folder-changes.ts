import type * as React from "react";

export type FolderChanges = React.MutableRefObject<{
  notes: Map<string, number>;
  deletedProjects: Map<string, number>;
}>;

export function markFolderChange(changes: Map<string, number>, id: string) {
  changes.set(id, (changes.get(id) ?? 0) + 1);
}
