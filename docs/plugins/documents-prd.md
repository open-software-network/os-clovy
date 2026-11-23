# PRD: Documents plugin

- **Mode:** CEO
- **Rank:** 9 of 10
- **Score:** 67/100
- **Date:** 2026-07-13
- **Status:** Proposed

## Thesis

Documents turns Clovy's strongest raw material - conversations, transcripts,
notes, and connected context - into a finished local artifact. It should create
and revise professional documents without requiring a cloud document service
and without overwriting the user's original.

It ranks ninth because the value is broad and highly aligned with local-first
privacy, but high-fidelity office formats and rendering require a new artifact
foundation. That foundation should be proven before presentations or richer
design outputs.

## Customer and problem

Clovy users leave meetings with the information needed for a brief, proposal,
memo, decision record, or client follow-up, but still assemble the document by
hand. Generic chat output is not a finished deliverable: it lacks structure,
consistent styles, page-aware verification, editable format, and source links.

## Product promise

Ask Clovy for a finished document. It builds an editable local artifact from the
sources you chose, shows a preview and validation report, and exports only when
you approve the destination.

## V1 experience

- Start from a Clovy note, selected local files, or connected sources.
- Choose a template or describe the document: memo, brief, proposal, decision
  record, agenda, or meeting recap.
- Clovy creates a versioned artifact in its workspace and shows a rendered
  preview plus outline, page/section count, and warnings.
- Ask for revisions in chat; every revision creates a recoverable version.
- Export DOCX, Markdown, or plain text through a native save dialog.
- Import an existing supported document, edit a copy, and compare the new
  version without changing the original.

## Scope

### V1

- Create and read Markdown, plain text, and a supported DOCX subset.
- Supported DOCX structures: paragraphs, headings, lists, tables, links,
  images, headers/footers, page breaks, and basic styles.
- Semantic validation and an honest rendered preview for supported structures.
- Version history, source references, templates, and native export.

### Later

- PDF export, comments, tracked changes/redlines, footnotes, fields, complex
  pagination, collaborative cloud publishing, and presentations.

## Non-goals

- Replacing a full word processor.
- Pixel-perfect round-trip editing of arbitrary DOCX files.
- Running macros, embedded objects, or external links automatically.
- Overwriting the imported original.
- Uploading documents to a Clovy-hosted storage service.

## Packaging

- No connector is required.
- App-owned toolset: document inspect, create, revise, render, validate, export.
- Skills: executive memo, client brief, proposal, decision record, agenda,
  meeting recap.
- Optional connectors provide source context or a later publication target.

## Privacy and trust

Artifact storage and transformation stay on-device. Content needed for drafting
still follows the selected inference path. The plugin listing must distinguish
local file transformation from model inference. Imported content is untrusted,
and external relationships, macros, and active content are disabled.

Exports use a user-selected destination. Existing files require an explicit
replace confirmation from the native dialog; the normal path creates a new file.

## Business model

Core creation and export are Hobby. Larger documents, premium template packs,
and high-frequency cross-plugin artifact routines can be Pro. Model usage is
already metered; local render/validation should not add a billing slug.

## Success measures

| Metric | Target |
| --- | ---: |
| Started documents reaching export | 45% |
| Exports opened successfully in a reference editor | 99% |
| Exported documents requiring structural repair | under 5% |
| Users creating a second document within 30 days | 35% |
| Original source files overwritten without explicit confirmation | 0 |

## Risks and gates

- Office formats contain far more features than V1 supports; silent loss is
  worse than an explicit unsupported warning.
- Visual layout can differ across editors and fonts.
- A converter or rendering sidecar adds binary size, licensing, sandbox, and
  supply-chain risk.
- Source documents can contain prompt injection and active content.

## Decision requested

Approve a supported-subset, versioned Documents plugin with DOCX/Markdown/text
creation and copy-on-edit. Gate implementation on the artifact-engine spike and
defer redlines and PDF export.
