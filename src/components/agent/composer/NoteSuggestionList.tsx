import {
  forwardRef,
  useCallback,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { IconNoteText } from "central-icons/IconNoteText";

import { intlLocale, type TFunction, useT } from "../../../i18n";
import type { NoteListItemDto } from "../../../lib/tauri";
import { useScrollFade } from "../../../lib/use-scroll-fade";

export type NoteSuggestionListProps = {
  items: NoteListItemDto[];
  command: (item: NoteListItemDto) => void;
};

/** Compact date for the palette's right rail — month + day is enough to place
 * a note without crowding the row. */
function formatSuggestionDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(intlLocale(), { month: "short", day: "numeric" }).format(date);
}

/** One-line label for a row: lead with whatever identifies the note — its
 * title, else its content preview, else the "New note" placeholder — and show
 * the preview inline (to the right) only when it distinctly adds to a real
 * title. Never duplicate the primary, so an empty note doesn't read its
 * placeholder twice, and rows stay a single, uniform line. */
function noteSuggestionLabels(
  t: TFunction,
  item: NoteListItemDto,
): { primary: string; secondary: string } {
  const title = item.title.trim();
  const preview = item.preview.trim();
  const primary = title || preview || t("chat.composer.newNote");
  const secondary = title && preview && preview !== title ? preview : "";
  return { primary, secondary };
}

export type NoteSuggestionListHandle = {
  onKeyDown: (event: KeyboardEvent) => boolean;
};

/** The floating palette that opens when the user types "@" in the composer.
 * It mirrors the slash palette's keyboard contract but only lists notes. */
export const NoteSuggestionList = forwardRef<NoteSuggestionListHandle, NoteSuggestionListProps>(
  ({ items, command }, ref) => {
    const t = useT();
    const [selected, setSelected] = useState(0);
    const [activeSource, setActiveSource] = useState<"keyboard" | "pointer" | null>(null);
    const menuRef = useRef<HTMLDivElement | null>(null);
    const fade = useScrollFade(menuRef);
    const itemIds = items.map((item) => item.id).join("\u0000");
    const lastItemIdsRef = useRef(itemIds);

    useLayoutEffect(() => {
      // Reset the highlight whenever filtering changes the result set so Enter
      // always targets a visible row.
      if (lastItemIdsRef.current !== itemIds) {
        lastItemIdsRef.current = itemIds;
        setSelected(0);
        setActiveSource(null);
      }
      fade.update();
    }, [itemIds, fade.update]);

    const choose = useCallback(
      (index: number) => {
        const item = items[index];
        if (item) command(item);
      },
      [items, command],
    );

    useImperativeHandle(
      ref,
      () => ({
        onKeyDown: (event) => {
          if (items.length === 0) return false;
          if (event.key === "ArrowDown") {
            setSelected((index) => {
              setActiveSource("keyboard");
              return (index + 1) % items.length;
            });
            return true;
          }
          if (event.key === "ArrowUp") {
            setSelected((index) => {
              setActiveSource("keyboard");
              return (index - 1 + items.length) % items.length;
            });
            return true;
          }
          if (event.key === "Enter" || event.key === "Tab") {
            choose(selected);
            return true;
          }
          return false;
        },
      }),
      [items, selected, choose],
    );

    if (items.length === 0) {
      return (
        <div className="agent-category-menu agent-category-menu-empty">
          {t("chat.composer.noNotesFound")}
        </div>
      );
    }

    return (
      <div className="agent-category-menu-shell agent-note-suggestion-menu-shell">
        <div className="agent-category-menu-scroll-wrap scroll-fade" {...fade.props}>
          <div
            ref={menuRef}
            className="agent-category-menu agent-note-suggestion-menu"
            role="listbox"
            aria-label={t("chat.composer.referenceNote")}
            onScroll={() => {
              fade.update();
            }}
          >
            {items.map((item, index) => {
              const { primary, secondary } = noteSuggestionLabels(t, item);
              const date = formatSuggestionDate(item.updatedAt);
              return (
                <button
                  key={item.id}
                  type="button"
                  role="option"
                  // Name the option by its leading label; the inline preview +
                  // date are visual context, not part of the accessible name.
                  aria-label={primary}
                  aria-selected={index === selected}
                  data-active={activeSource && index === selected ? true : undefined}
                  onMouseDown={(event) => {
                    event.preventDefault();
                    choose(index);
                  }}
                  onMouseEnter={() => {
                    setSelected(index);
                    setActiveSource("pointer");
                  }}
                  onFocus={() => {
                    setSelected(index);
                    setActiveSource("keyboard");
                  }}
                >
                  <span className="agent-category-menu-icon agent-note-suggestion-menu-icon">
                    <IconNoteText size={16} aria-hidden />
                  </span>
                  <span className="agent-category-menu-copy agent-note-suggestion-menu-copy">
                    <span className="agent-category-menu-label agent-note-suggestion-menu-label">
                      {primary}
                    </span>
                    {secondary ? (
                      <span className="agent-note-suggestion-menu-preview">{secondary}</span>
                    ) : null}
                  </span>
                  {date ? <span className="agent-note-suggestion-menu-date">{date}</span> : null}
                </button>
              );
            })}
          </div>
        </div>
      </div>
    );
  },
);
NoteSuggestionList.displayName = "NoteSuggestionList";
