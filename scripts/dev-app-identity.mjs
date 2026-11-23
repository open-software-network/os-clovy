const ISOLATED_DEV_NAMESPACES = new Set(["codex", "claude"]);
const IDENTIFIER = /^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/;

export function devAppIdentityForBranch(
  branchName,
  { baseName = "Clovy", baseIdentifier = "dev.guilhermexp.clovy", identifierOverride } = {},
) {
  // CLOVY_DEV_APP_IDENTIFIER: an explicit, isolated compiled identity (and so
  // an isolated `<identifier>-dev` data directory), e.g. for native proofs.
  const override = `${identifierOverride ?? ""}`.trim();
  if (override) {
    if (!IDENTIFIER.test(override)) {
      throw new Error(`Invalid CLOVY_DEV_APP_IDENTIFIER: ${override}`);
    }
    return { productName: baseName, identifier: override };
  }

  const normalized = `${branchName ?? ""}`.trim();
  const namespace = normalized.split("/", 1)[0]?.toLowerCase();
  const issueMatch = normalized.match(/\bjun-(\d+)\b/i);

  if (!ISOLATED_DEV_NAMESPACES.has(namespace) || !issueMatch) {
    return { productName: baseName, identifier: baseIdentifier };
  }

  const issueNumber = issueMatch[1];
  return {
    productName: baseName,
    identifier: `${baseIdentifier}.${namespace}.jun${issueNumber}`,
  };
}
