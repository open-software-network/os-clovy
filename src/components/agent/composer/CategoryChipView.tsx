import { NodeViewWrapper, type NodeViewProps } from "@tiptap/react";

import { useLocale } from "../../../i18n";
import { CategoryIcon } from "./CategoryIcon";
import { reportCategoryDef, type ReportCategory } from "./reportCategory";

export function CategoryChipView({ node }: NodeViewProps) {
  // Subscribes to the language so the translated label re-renders.
  useLocale();
  const category = node.attrs.category as ReportCategory;
  const def = reportCategoryDef(category);

  return (
    <NodeViewWrapper
      as="span"
      className="agent-category-chip"
      data-category={category}
      contentEditable={false}
    >
      <span className="agent-category-chip-icon" aria-hidden="true">
        <CategoryIcon category={category} size={10} filled />
      </span>
      {def?.label ?? ""}
    </NodeViewWrapper>
  );
}
