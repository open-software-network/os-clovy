import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { Icon3dBoxTop } from "central-icons/Icon3dBoxTop";
import { IconBuildingBlocks } from "central-icons/IconBuildingBlocks";
import { IconCat } from "central-icons/IconCat";
import { IconFileText } from "central-icons/IconFileText";
import { IconVideoClip } from "central-icons/IconVideoClip";

import { useT } from "../../../i18n";
import type { AgentSkillInfo } from "../../../lib/tauri";
import type { BuiltinComposerSlashCommandDef } from "../../../lib/agent-composer-slash-commands";
import { useScrollFade } from "../../../lib/use-scroll-fade";

const SKILL_DETAIL_HOVER_INTENT_MS = 150;

export type ComposerSlashCommandItem =
  | { kind: "builtin"; command: BuiltinComposerSlashCommandDef }
  | { kind: "skill"; skill: AgentSkillInfo };

export type CategorySuggestionListProps = {
  items: ComposerSlashCommandItem[];
  command: (item: ComposerSlashCommandItem) => void;
};

export type CategorySuggestionListHandle = {
  onKeyDown: (event: KeyboardEvent) => boolean;
};

/** The floating palette that opens when the user types "/" in the composer.
 * Mirrors the os-platform mention list: arrow keys move the highlight, Enter
 * or Tab commits, and the editor's suggestion plugin owns mount/teardown. */
export const CategorySuggestionList = forwardRef<
  CategorySuggestionListHandle,
  CategorySuggestionListProps
>(({ items, command }, ref) => {
  const t = useT();
  const [selected, setSelected] = useState(0);
  const [activeSource, setActiveSource] = useState<"keyboard" | "pointer" | null>(null);
  const [detail, setDetail] = useState<{
    index: number;
    key: string;
    top: number;
    side: "left" | "right";
  } | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const fade = useScrollFade(menuRef);
  const rowRefs = useRef(new Map<number, HTMLButtonElement>());
  const hoverTimerRef = useRef<number | null>(null);
  const cancelHoverIntent = useCallback(() => {
    if (hoverTimerRef.current !== null) {
      window.clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);
  const hoverIntent = useCallback(
    (action: () => void) => {
      cancelHoverIntent();
      hoverTimerRef.current = window.setTimeout(action, SKILL_DETAIL_HOVER_INTENT_MS);
    },
    [cancelHoverIntent],
  );
  // Snap the highlight back to the top whenever the filtered set changes so a
  // press of Enter never targets a row that just scrolled out of the results.
  useEffect(() => {
    cancelHoverIntent();
    setSelected(0);
    setActiveSource(null);
    setDetail(null);
  }, [items, cancelHoverIntent]);

  useEffect(() => cancelHoverIntent, [cancelHoverIntent]);

  // Re-measure the fades when filtering changes the list's content.
  useLayoutEffect(() => {
    fade.update();
  }, [items, fade.update]);

  const choose = useCallback(
    (index: number) => {
      const item = items[index];
      if (item) command(item);
    },
    [items, command],
  );

  const showCommandDetail = useCallback(
    (index: number, row = rowRefs.current.get(index) ?? null) => {
      const item = items[index];
      const menu = menuRef.current;
      if (!item || !menu || !row || !commandHasDetail(item)) {
        setDetail(null);
        return;
      }

      const menuRect = menu.getBoundingClientRect();
      const rowRect = row.getBoundingClientRect();
      const gap = 6;
      const margin = 12;
      const cardWidth = 300;
      const canOpenRight = menuRect.right + gap + cardWidth <= window.innerWidth - margin;
      const canOpenLeft = menuRect.left - gap - cardWidth >= margin;
      const side = canOpenRight ? "right" : canOpenLeft ? "left" : null;
      if (!side) {
        setDetail(null);
        return;
      }

      const cardHeightGuess = 150;
      const maxTop = Math.max(0, window.innerHeight - menuRect.top - margin - cardHeightGuess);
      setDetail({
        index,
        key: commandItemKey(item),
        side,
        top: Math.min(Math.max(rowRect.top - menuRect.top, 0), maxTop),
      });
    },
    [items],
  );

  useImperativeHandle(
    ref,
    () => ({
      onKeyDown: (event) => {
        if (items.length === 0) return false;
        if (event.key === "ArrowDown") {
          setSelected((index) => {
            const next = (index + 1) % items.length;
            setActiveSource("keyboard");
            cancelHoverIntent();
            window.requestAnimationFrame(() => showCommandDetail(next));
            return next;
          });
          return true;
        }
        if (event.key === "ArrowUp") {
          setSelected((index) => {
            const next = (index - 1 + items.length) % items.length;
            setActiveSource("keyboard");
            cancelHoverIntent();
            window.requestAnimationFrame(() => showCommandDetail(next));
            return next;
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
    [items, selected, choose, showCommandDetail, cancelHoverIntent],
  );

  if (items.length === 0) {
    return (
      <div className="agent-category-menu agent-category-menu-empty">
        {t("chat.composer.noMatches")}
      </div>
    );
  }

  const detailItem = detail ? items[detail.index] : undefined;
  const activeDetail =
    detail && detail.index === selected && detailItem && commandItemKey(detailItem) === detail.key
      ? detailItem
      : null;
  const builtins = items
    .map((item, index) => ({ item, index }))
    .filter(({ item }) => item.kind === "builtin");
  const skills = items
    .map((item, index) => ({ item, index }))
    .filter(({ item }) => item.kind === "skill");

  return (
    <div
      className="agent-category-menu-shell"
      onMouseLeave={() => {
        cancelHoverIntent();
        setActiveSource(null);
        setDetail(null);
      }}
    >
      <div className="agent-category-menu-scroll-wrap scroll-fade" {...fade.props}>
        <div
          ref={menuRef}
          className="agent-category-menu"
          role="listbox"
          aria-label={t("chat.composer.slashCommands")}
          onScroll={() => {
            fade.update();
            setDetail(null);
          }}
        >
          {builtins.length ? (
            <div className="agent-category-menu-section" role="presentation">
              <div className="agent-category-menu-section-label">{t("chat.composer.commands")}</div>
              {builtins.map(({ item, index }) => renderCommandRow(item, index))}
            </div>
          ) : null}
          {skills.length ? (
            <div className="agent-category-menu-section" role="presentation">
              <div className="agent-category-menu-section-label">{t("chat.composer.skills")}</div>
              {skills.map(({ item, index }) => renderCommandRow(item, index))}
            </div>
          ) : null}
        </div>
      </div>
      {activeDetail && commandHasDetail(activeDetail) ? (
        <div
          className="agent-category-menu-detail-card"
          data-side={detail?.side}
          style={{ top: detail?.top ?? 0 }}
        >
          <p className="agent-category-menu-detail-title">{commandItemDetailTitle(activeDetail)}</p>
          {commandItemDetailMeta(activeDetail) ? (
            <p className="agent-category-menu-detail-meta">{commandItemDetailMeta(activeDetail)}</p>
          ) : null}
          {commandItemDetailDescription(activeDetail) ? (
            <p className="agent-category-menu-detail-desc">
              {commandItemDetailDescription(activeDetail)}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );

  function renderCommandRow(item: ComposerSlashCommandItem, index: number) {
    return (
      <button
        key={commandItemKey(item)}
        ref={(node) => {
          if (node) rowRefs.current.set(index, node);
          else rowRefs.current.delete(index);
        }}
        type="button"
        role="option"
        aria-selected={index === selected}
        data-active={activeSource && index === selected ? true : undefined}
        data-kind={item.kind}
        // mousedown (not click) so the press commits before the editor's
        // blur can tear the popover down.
        onMouseDown={(event) => {
          event.preventDefault();
          choose(index);
        }}
        onMouseEnter={(event) => {
          setSelected(index);
          setActiveSource("pointer");
          const row = event.currentTarget;
          hoverIntent(() => showCommandDetail(index, row));
        }}
        onFocus={(event) => {
          setSelected(index);
          setActiveSource("keyboard");
          cancelHoverIntent();
          showCommandDetail(index, event.currentTarget);
        }}
      >
        <span className="agent-category-menu-icon">
          {item.kind === "builtin" ? (
            commandItemIcon(item)
          ) : (
            <IconBuildingBlocks size={16} aria-hidden />
          )}
        </span>
        <span className="agent-category-menu-copy">
          <span className="agent-category-menu-label">{commandItemLabel(item)}</span>
        </span>
      </button>
    );
  }
});
CategorySuggestionList.displayName = "CategorySuggestionList";

function commandItemKey(item: ComposerSlashCommandItem) {
  if (item.kind === "builtin") return `builtin:${item.command.name}`;
  return `skill:${item.skill.name}`;
}

function commandItemLabel(item: ComposerSlashCommandItem) {
  if (item.kind === "builtin") return item.command.label;
  return item.skill.name;
}

function commandHasDetail(item: ComposerSlashCommandItem) {
  if (item.kind === "builtin") return Boolean(item.command.description.trim());
  if (item.kind === "skill") {
    return Boolean(item.skill.description?.trim() || item.skill.category?.trim());
  }
  return false;
}

function commandItemDetailTitle(item: ComposerSlashCommandItem) {
  if (item.kind === "builtin") return `/${item.command.name}`;
  if (item.kind === "skill") return item.skill.name;
}

function commandItemDetailMeta(item: ComposerSlashCommandItem) {
  if (item.kind === "skill") return item.skill.category?.trim() ?? "";
  return "";
}

function commandItemDetailDescription(item: ComposerSlashCommandItem) {
  if (item.kind === "builtin") return item.command.description.trim();
  if (item.kind === "skill") return item.skill.description?.trim() ?? "";
  return "";
}

function commandItemIcon(item: Extract<ComposerSlashCommandItem, { kind: "builtin" }>) {
  switch (item.command.name) {
    case "model":
      return <Icon3dBoxTop size={16} aria-hidden />;
    case "file":
      return <IconFileText size={16} aria-hidden />;
    case "image":
      return <IconCat size={16} aria-hidden />;
    case "video":
      return <IconVideoClip size={16} aria-hidden />;
  }
}
