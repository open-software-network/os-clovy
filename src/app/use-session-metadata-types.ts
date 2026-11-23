import type * as React from "react";

export type UseSessionMetadataDependencies = {
  appBlocked: boolean;
  bootstrapped: boolean;
  currentDataPartitionName: string;
  sessionFolderEditsRef: import("./session-folder-edits").SessionFolderEdits;
  folderChangesRef: import("./folder-changes").FolderChanges;
  sessionCompletionTouchedRef: React.MutableRefObject<Set<string>>;
  setCompletedSessions: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setSessionFolders: React.Dispatch<React.SetStateAction<Record<string, string[]>>>;
};
