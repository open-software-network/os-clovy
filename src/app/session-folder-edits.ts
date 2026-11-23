import type * as React from "react";

export type SessionFolderEdits = React.MutableRefObject<Map<string, number>>;

export function markSessionFolderEdited(edits: SessionFolderEdits, sessionId: string) {
  edits.current.set(sessionId, (edits.current.get(sessionId) ?? 0) + 1);
}
