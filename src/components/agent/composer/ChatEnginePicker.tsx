import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { type RefObject, useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { useT } from "../../../i18n";
import {
  type ChatEngineCatalog,
  type ChatEngineCliStatus,
  chatEngineCatalog,
  classifyChatEngineModel,
  defaultChatCliId,
} from "../../../lib/chat-engine";
import { RuntimeLogo } from "../../brand/RuntimeLogo";
import { useChatEnginePopoverPosition } from "./chat-engine-popover-position";

const DEFAULT_CLIS: ChatEngineCliStatus[] = [
  {
    id: "claude",
    name: "Claude Code",
    installed: false,
    reason: null,
    modelId: "__clovy_cli_engine__:claude",
  },
  {
    id: "codex",
    name: "Codex",
    installed: false,
    reason: null,
    modelId: "__clovy_cli_engine__:codex",
  },
  {
    id: "pi",
    name: "Pi",
    installed: false,
    reason: null,
    modelId: "__clovy_cli_engine__:pi",
  },
  {
    id: "agy",
    name: "Agy",
    installed: false,
    reason: null,
    modelId: "__clovy_cli_engine__:agy",
  },
  {
    id: "cursor-agent",
    name: "Cursor Agent",
    installed: false,
    reason: null,
    modelId: "__clovy_cli_engine__:cursor-agent",
  },
  {
    id: "copilot",
    name: "GitHub Copilot",
    installed: false,
    reason: null,
    modelId: "__clovy_cli_engine__:copilot",
  },
];

function fallbackCliName(id: string): string {
  switch (id) {
    case "claude":
      return "Claude Code";
    case "codex":
      return "Codex";
    case "pi":
      return "Pi";
    case "agy":
      return "Agy";
    case "cursor-agent":
      return "Cursor Agent";
    case "copilot":
      return "GitHub Copilot";
    default:
      return id;
  }
}

export type ChatEnginePickerProps = {
  model?: string;
  setModel: (modelId: string) => void;
  catalog?: ChatEngineCatalog | null;
  onRefreshCatalog?: () => Promise<ChatEngineCatalog | null>;
  anchorRef?: RefObject<HTMLElement | null>;
  readOnly?: boolean;
};

export function ChatEnginePicker({
  model,
  setModel,
  catalog: propCatalog,
  onRefreshCatalog,
  anchorRef,
  readOnly = false,
}: ChatEnginePickerProps) {
  const t = useT();
  const menuTitleId = useId();
  const [open, setOpen] = useState(false);
  const [internalCatalog, setInternalCatalog] = useState<ChatEngineCatalog | null>(
    propCatalog ?? null,
  );
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);

  const classification = classifyChatEngineModel(model, propCatalog ?? internalCatalog);
  const loadCatalog = useCallback(async () => {
    try {
      if (onRefreshCatalog) {
        const refreshed = await onRefreshCatalog();
        if (refreshed) {
          setInternalCatalog(refreshed);
          return;
        }
      }
      const fetched = await chatEngineCatalog();
      setInternalCatalog(fetched);
    } catch {
      // Fallback stays in place if IPC is unavailable.
    }
  }, [onRefreshCatalog]);

  useEffect(() => {
    if (propCatalog) {
      setInternalCatalog(propCatalog);
      return;
    }
    void loadCatalog();
  }, [loadCatalog, propCatalog]);

  useChatEnginePopoverPosition({
    open,
    triggerRef: triggerRef as RefObject<HTMLElement>,
    popoverRef: popoverRef as RefObject<HTMLElement>,
    anchorRef: (anchorRef ?? { current: null }) as RefObject<HTMLElement>,
  });

  useEffect(() => {
    if (!open) return;
    const handleMousedown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (popoverRef.current?.contains(target) || triggerRef.current?.contains(target)) {
        return;
      }
      setOpen(false);
    };
    const handleKeydown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault();
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    window.addEventListener("mousedown", handleMousedown);
    window.addEventListener("keydown", handleKeydown, true);
    return () => {
      window.removeEventListener("mousedown", handleMousedown);
      window.removeEventListener("keydown", handleKeydown, true);
    };
  }, [open]);

  const activeCatalog = propCatalog ?? internalCatalog;
  const clis = activeCatalog?.clis.length ? activeCatalog.clis : DEFAULT_CLIS;
  const defaultCliId = defaultChatCliId(clis);
  const activeEngineLogoId = classification.cli;
  const activeEngineLabel =
    clis.find((item) => item.id === classification.cli)?.name ??
    fallbackCliName(classification.cli);

  const handleSelectCli = (cli: ChatEngineCliStatus) => {
    if (!cli.installed) return;
    setOpen(false);
    setModel(cli.modelId);
  };

  const popoverContent = open ? (
    <div
      ref={popoverRef}
      className="agent-composer-model-popover agent-engine-popover"
      role="menu"
      aria-labelledby={menuTitleId}
    >
      <p id={menuTitleId} className="agent-composer-model-title">
        {t("chat.engine.menuTitle")}
      </p>

      <p className="agent-composer-model-title">{t("chat.engine.sectionClis")}</p>
      <div className="agent-composer-model-menu">
        {clis.map((cli) => {
          const isSelected = classification.kind === "cli" && classification.cli === cli.id;
          return (
            <button
              key={cli.id}
              type="button"
              className="agent-composer-model-row"
              role="menuitemradio"
              aria-checked={isSelected}
              disabled={!cli.installed}
              aria-disabled={!cli.installed}
              onClick={() => handleSelectCli(cli)}
            >
              <span className="agent-composer-model-row-logo" aria-hidden>
                <RuntimeLogo id={cli.id} name={cli.name} size={14} />
              </span>
              <span className="agent-composer-model-choice-copy">
                <span className="agent-composer-model-row-name">{cli.name}</span>
                {!cli.installed ? (
                  <span className="agent-composer-model-choice-desc">
                    {t("chat.engine.notFoundOnPath")}
                  </span>
                ) : cli.id === defaultCliId ? (
                  <span className="agent-composer-model-choice-desc">
                    {t("chat.engine.defaultEngine")}
                  </span>
                ) : null}
              </span>
              {isSelected ? (
                <IconCheckmark2Small
                  size={14}
                  aria-hidden
                  className="agent-composer-model-row-check"
                />
              ) : null}
            </button>
          );
        })}
      </div>
    </div>
  ) : null;

  return (
    <>
      <div className="agent-composer-model" data-engine-picker="true" data-open={open || undefined}>
        <button
          ref={triggerRef}
          type="button"
          className="agent-composer-model-trigger"
          aria-label={t("chat.engine.pickerLabel", { name: activeEngineLabel })}
          aria-haspopup="menu"
          aria-expanded={open}
          disabled={readOnly}
          onClick={() => {
            if (readOnly) return;
            if (!open) {
              void loadCatalog();
              setOpen(true);
            } else {
              setOpen(false);
            }
          }}
        >
          {activeEngineLogoId ? (
            <RuntimeLogo id={activeEngineLogoId} name={activeEngineLabel} size={14} decorative />
          ) : null}
          <span className="agent-composer-model-trigger-name">{activeEngineLabel}</span>
          <IconChevronDownSmall size={12} aria-hidden />
        </button>
      </div>
      {open && anchorRef?.current
        ? createPortal(popoverContent, anchorRef.current)
        : popoverContent}
    </>
  );
}
