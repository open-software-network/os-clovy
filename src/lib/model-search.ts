import type { ProviderModelDto } from "./tauri";

/** Matches catalog metadata as ordered tokens so punctuation and intervening
 * version terms do not make natural model queries fail. */
export function modelMatchesQuery(model: ProviderModelDto, query: string) {
  const queryTokens = searchTokens(query);
  if (!queryTokens.length) return false;

  const modelTokens = [
    model.name,
    model.id,
    model.description,
    model.privacy,
    ...model.traits,
  ].flatMap(searchTokens);
  let modelIndex = 0;
  return queryTokens.every((queryToken) => {
    while (modelIndex < modelTokens.length && !modelTokens[modelIndex].includes(queryToken)) {
      modelIndex += 1;
    }
    if (modelIndex === modelTokens.length) return false;
    modelIndex += 1;
    return true;
  });
}

function searchTokens(value: unknown): string[] {
  return typeof value === "string" ? (value.toLowerCase().match(/[a-z0-9]+/g) ?? []) : [];
}
