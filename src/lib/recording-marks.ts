import type { RecordingMarkDto } from "./tauri";

const listeners = new Set<(mark: RecordingMarkDto) => void>();

export function subscribeRecordingMarks(listener: (mark: RecordingMarkDto) => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function publishRecordingMark(mark: RecordingMarkDto) {
  for (const listener of listeners) listener(mark);
}
