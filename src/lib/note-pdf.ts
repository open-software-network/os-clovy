import { t } from "../i18n/translate";
import { printCurrentWebview } from "./tauri";

/**
 * Open the platform print sheet with a useful default PDF filename.
 *
 * The native print sheet exposes Save as PDF on macOS and the equivalent PDF
 * destination on other platforms. Clovy invokes it through Tauri because
 * WKWebView does not implement `window.print()`.
 */
type ExportNoteAsPdfOptions = {
  showNotes?: () => void | Promise<void>;
  waitForPaint?: () => void | Promise<void>;
  print?: () => void | Promise<void>;
};

export async function exportNoteAsPdf(
  noteTitle: string,
  {
    showNotes,
    waitForPaint = () =>
      new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve())),
    print = printCurrentWebview,
  }: ExportNoteAsPdfOptions = {},
) {
  if (showNotes) {
    await showNotes();
    await waitForPaint();
  }

  const previousTitle = document.title;
  document.title = noteTitle.trim() || t("lib.notePdf.defaultTitle");

  try {
    await print();
  } finally {
    document.title = previousTitle;
  }
}
