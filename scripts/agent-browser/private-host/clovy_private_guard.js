function _clovyGuard(
  doc,
  documentId,
  backendNodeId,
  expected,
  operation,
  text,
  key,
  expectedRaw,
  redactedValues,
  metadataId,
) {
  if (!this.isConnected || this.ownerDocument !== doc || doc !== document)
    return { error: "stale_reference" };

  const safeText = (node) => {
    if (node.nodeType === 3) return node.nodeValue || "";
    if (node.nodeType !== 1) return "";
    if (node.matches("input,textarea,select,[contenteditable]") || node.isContentEditable)
      return "";
    return Array.from(node.childNodes).map(safeText).join("");
  };
  const tree = this.getRootNode();
  const referencedLabel = (this.getAttribute("aria-labelledby") || "")
    .split(/\s+/)
    .filter(Boolean)
    .map((id) => tree.getElementById?.(id) || doc.getElementById(id))
    .filter((node) => node && node !== this)
    .map(safeText)
    .join(" ")
    .trim();
  const explicitLabel = Array.from(this.labels || [])
    .map(safeText)
    .join(" ")
    .trim();
  const tag = this.tagName.toLowerCase();
  const inputType = tag === "input" ? this.type.toLowerCase() : "";
  const role =
    this.getAttribute("role") ||
    { a: "link", button: "button", textarea: "textbox", select: "combobox" }[tag] ||
    (tag === "input"
      ? { button: "button", submit: "button", checkbox: "checkbox", radio: "radio" }[inputType] ||
        "textbox"
      : "");
  const label =
    referencedLabel ||
    (this.getAttribute("aria-label") || "").trim() ||
    explicitLabel ||
    this.getAttribute("alt") ||
    this.getAttribute("title") ||
    (["input", "textarea", "select"].includes(tag) || this.isContentEditable
      ? ""
      : safeText(this).trim());
  const name = this.getAttribute("name") || "";
  const rawElement = {
    tag,
    inputType,
    role,
    name,
    id: this.id || "",
    label,
    autocomplete: this.getAttribute("autocomplete") || "",
    inForm: !!(this.form || this.closest("form")),
    contentEditable: this.isContentEditable,
  };
  // Authoritative host-only interaction class; the broker consumes this closed enum.
  // Classify ORIGINAL facts, before value-derived metadata is redacted.
  const asciiLower = (value) => value.replace(/[A-Z]/g, (c) => c.toLowerCase());
  const sensitiveText = asciiLower(
    [rawElement.name, rawElement.id, label, rawElement.role].join(" "),
  ).replace(/[_-]/g, " ");
  const sensitiveTerms = [
    "password",
    "passcode",
    "one time code",
    "one-time code",
    "otp",
    "verification code",
    "authentication code",
    "auth code",
    "2fa code",
    "2 fa code",
    "mfa code",
    "two factor code",
    "security code",
    "card number",
    "credit card",
    "expiration",
    "expiry",
    "exp date",
    "exp month",
    "exp year",
    "mm/yy",
    "mm / yy",
    "valid thru",
    "valid through",
    "cvv",
    "cvc",
  ];
  const humanOnly =
    inputType === "password" ||
    asciiLower(rawElement.autocomplete)
      .split(/\s+/)
      .some(
        (token) =>
          token.includes("password") || token === "one-time-code" || token.startsWith("cc-"),
      ) ||
    sensitiveTerms.some((term) => sensitiveText.includes(term));
  const secrets = [...new Set((redactedValues || []).filter((value) => value))].sort(
    (a, b) => b.length - a.length,
  );
  const buckets = new Map();
  for (const secret of secrets) {
    const first = secret[0];
    if (!buckets.has(first)) buckets.set(first, []);
    buckets.get(first).push(secret);
  }
  let budget = 16 * 1024 * 1024;
  let outputBudget = 1024 * 1024;
  const redact = (value) => {
    if (value.length > 1024 * 1024) return null;
    const safe = [];
    let coveredUntil = 0;
    let masking = false;
    for (let i = 0; i < value.length; ) {
      const size = value.codePointAt(i) > 0xffff ? 2 : 1;
      budget -= size;
      if (budget < 0) return null;
      for (const secret of buckets.get(value[i]) || []) {
        budget -= secret.length;
        if (budget < 0) return null;
        if (value.startsWith(secret, i)) {
          if (!masking) {
            outputBudget -= "[redacted]".length;
            if (outputBudget < 0) return null;
            safe.push("[redacted]");
            masking = true;
          }
          coveredUntil = Math.max(coveredUntil, i + secret.length);
          break;
        }
      }
      if (i >= coveredUntil) {
        masking = false;
        outputBudget -= size;
        if (outputBudget < 0) return null;
        safe.push(value.slice(i, i + size));
      }
      i += size;
    }
    return safe.join("");
  };
  const safeName = redact(rawElement.name);
  const safeId = redact(rawElement.id);
  const safeLabel = redact(rawElement.label);
  const safeRole = redact(rawElement.role);
  const safeAutocomplete = redact(rawElement.autocomplete);
  if ([safeName, safeId, safeLabel, safeRole, safeAutocomplete].some((v) => v === null))
    return { error: "redaction_failed" };
  const element = {
    ...rawElement,
    name: safeName,
    id: safeId,
    label: safeLabel.slice(0, 1000),
    role: safeRole,
    autocomplete: safeAutocomplete,
  };
  const inspection = {
    url: location.href,
    origin: location.origin,
    documentId,
    metadataId,
    interactionClass: humanOnly ? "human_only" : "approval_required",
    backendNodeId,
    element,
  };
  if (!expected) return { inspection, rawElement };
  const equal = (a, b) =>
    a === b ||
    (a &&
      b &&
      typeof a === "object" &&
      typeof b === "object" &&
      Object.keys(a).length === Object.keys(b).length &&
      Object.keys(a).every((k) => equal(a[k], b[k])));
  if (!equal(inspection, expected) || !equal(rawElement, expectedRaw))
    return { error: "stale_reference" };
  if (humanOnly) return { error: "sensitive_field" };
  const verify = () => {
    const check = _clovyGuard.call(
      this,
      doc,
      documentId,
      backendNodeId,
      null,
      null,
      null,
      null,
      null,
      redactedValues,
      metadataId,
    );
    return (
      check.inspection && equal(check.inspection, expected) && equal(check.rawElement, expectedRaw)
    );
  };
  if (operation === "click") {
    this.click();
    return { inspection };
  }
  if (operation === "fill") {
    if (
      (!["input", "textarea"].includes(tag) && !this.isContentEditable) ||
      this.disabled ||
      this.readOnly ||
      (tag === "input" &&
        ["hidden", "file", "checkbox", "radio", "button", "submit", "reset", "image"].includes(
          inputType,
        ))
    )
      return { error: "unsupported_operation" };
    this.focus();
    if (!verify() || this.getRootNode().activeElement !== this || this.readOnly || this.disabled)
      return { error: "effect_unknown" };
    if (this.isContentEditable) this.textContent = text;
    else {
      const setter = Object.getOwnPropertyDescriptor(
        tag === "input" ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype,
        "value",
      ).set;
      setter.call(this, text);
    }
    this.dispatchEvent(
      new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }),
    );
    this.dispatchEvent(new Event("change", { bubbles: true }));
    return { inspection };
  }
  if (operation === "press") {
    if (!["Enter", "Space", "Escape", "Backspace", "Delete"].includes(key))
      return { error: "unsupported_key" };
    const textControl =
      tag === "textarea" ||
      (tag === "input" &&
        ["text", "search", "tel", "url", "email", "password"].includes(inputType));
    const editing =
      key === "Backspace" ||
      key === "Delete" ||
      (key === "Space" && (textControl || this.isContentEditable));
    if (editing && (this.readOnly || this.disabled)) return { error: "unsupported_operation" };
    const originalForm = this.form;
    this.focus();
    if (!verify() || this.getRootNode().activeElement !== this) return { error: "effect_unknown" };
    if (this.form !== originalForm) return { error: "effect_unknown" };
    if (editing && (this.readOnly || this.disabled)) return { error: "effect_unknown" };
    const actual = key === "Space" ? " " : key;
    const proceed = this.dispatchEvent(
      new KeyboardEvent("keydown", { key: actual, bubbles: true, cancelable: true }),
    );
    if (!verify() || this.getRootNode().activeElement !== this) return { error: "effect_unknown" };
    if (this.form !== originalForm) return { error: "effect_unknown" };
    if (editing && (this.readOnly || this.disabled)) return { error: "effect_unknown" };
    if (proceed && key === "Enter") {
      if (tag === "button" || tag === "a" || ["submit", "button"].includes(inputType)) this.click();
      else if (this.form && tag !== "textarea") {
        const submitter = Array.from(this.form.getRootNode().querySelectorAll("button,input")).find(
          (control) =>
            control.form === this.form &&
            ((control.tagName === "BUTTON" && control.type === "submit") ||
              (control.tagName === "INPUT" && ["submit", "image"].includes(control.type))),
        );
        if (submitter) {
          if (!submitter.matches(":disabled")) submitter.click();
        } else {
          const blocking = Array.from(this.form.elements).filter(
            (control) =>
              control.tagName === "INPUT" &&
              [
                "text",
                "search",
                "tel",
                "url",
                "email",
                "password",
                "date",
                "month",
                "week",
                "time",
                "datetime-local",
                "number",
              ].includes(control.type),
          );
          if (blocking.length <= 1) HTMLFormElement.prototype.requestSubmit.call(this.form);
        }
      }
    } else if (
      proceed &&
      key === "Space" &&
      (tag === "button" || ["checkbox", "radio", "button", "submit"].includes(inputType))
    )
      this.click();
    else if (proceed && editing && (textControl || this.isContentEditable)) {
      if (this.isContentEditable) {
        const selection = doc.getSelection();
        if (
          !selection?.rangeCount ||
          !this.contains(selection.anchorNode) ||
          !this.contains(selection.focusNode)
        )
          return { error: "effect_unknown" };
      } else if (
        inputType !== "email" &&
        (this.selectionStart === null || this.selectionEnd === null)
      ) {
        return { error: "effect_unknown" };
      }
      const editInputType =
        key === "Space"
          ? "insertText"
          : key === "Delete"
            ? "deleteContentForward"
            : "deleteContentBackward";
      const before = this.dispatchEvent(
        new InputEvent("beforeinput", {
          bubbles: true,
          cancelable: true,
          inputType: editInputType,
          data: key === "Space" ? " " : null,
        }),
      );
      if (!verify() || this.getRootNode().activeElement !== this || this.readOnly || this.disabled)
        return { error: "effect_unknown" };
      if (this.isContentEditable) {
        const selection = doc.getSelection();
        if (
          !selection?.rangeCount ||
          !this.contains(selection.anchorNode) ||
          !this.contains(selection.focusNode)
        )
          return { error: "effect_unknown" };
      }
      if (
        before &&
        !doc.execCommand(
          key === "Space" ? "insertText" : key === "Delete" ? "forwardDelete" : "delete",
          false,
          key === "Space" ? " " : undefined,
        )
      )
        return { error: "effect_unknown" };
    }
    this.dispatchEvent(new KeyboardEvent("keyup", { key: actual, bubbles: true }));
    return { inspection };
  }
  return { error: "unsupported_operation" };
}
