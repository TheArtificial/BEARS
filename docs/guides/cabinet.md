# Cabinet: shared knowledge

Cabinet is your Den's shared knowledge wiki. People and Bears read and edit the
same items; every edit publishes a new immutable revision, and the full
revision history is always available. There is one Cabinet per Den.

Contract and design: [cabinet-contract.md](../architecture/cabinet-contract.md).
Plan and phase status: [CABINET_IMPLEMENTATION_PLAN.md](../roadmap/CABINET_IMPLEMENTATION_PLAN.md).

## Using the wiki (people)

Open **`/cabinet`** while logged in.

- **Browse and search** — the index searches titles and current content.
  Archived items are hidden by default (`Show archived items` toggles).
- **Create** — `/cabinet/new`: a title plus a Markdown body. The item is
  visible to everyone in the Den (and to Cabinet-enabled Bears) immediately.
- **Edit** — every save publishes a new revision. If someone (or some Bear)
  published a newer revision while you were editing, your save is refused and
  the form comes back with your draft preserved and the conflict explained —
  review the latest revision, fold your change in, and save again. Nothing is
  ever merged silently and nothing is overwritten.
- **History** — every revision is immutable and permanently viewable at
  `/cabinet/{item}/history`, with author kind and timestamp.
- **Archive / restore** — archiving hides an item from default search and
  blocks edits until restored. Deletion is a tombstone: revision history stays
  citable. There is no hard delete in the UI.

## What Bears can do

Bears use the same knowledge store through four tools: `cabinet_search`,
`cabinet_read` (all stances), `cabinet_create`, `cabinet_update`
(`chat`/`pair`/`curate` stances). A Bear's edits go through exactly the same
facade, versioning, and conflict rules as yours, and show up in history as
Bear-authored with the acting stance.

Cabinet is deliberately separate from a Bear's private memory: memory tools
cannot write Cabinet, and Cabinet tools cannot write Bear memory.

## Permissions (Phase 1)

- **People:** every logged-in Den user can read, create, and edit every item —
  the open-wiki default.
- **Bears:** gated per Bear by the `cabinet_enabled` flag on the Bear record
  (default on, like `work_enabled`). A disabled Bear does not see the Cabinet
  tools and the server independently refuses it access.
- **Missions and collections** (scoped sharing, review/approval policy) are
  Phase 2: item requests that name them are refused with a clear policy error
  today.

## Source links

An item can carry provenance links to material outside Cabinet: web URLs,
offline sources (synthetic schemes such as `book://isbn/…`), artifact refs,
or conversations. Links are provenance only — Cabinet never fetches or stores
the bytes behind them.

## Limitations

- Search is substring matching over titles and current content (no semantic
  recall yet; that is Phase 3, via the derived recall index).
- Attachments (files on items, via artifact refs) are Phase 3.
- The editor is a plain Markdown textarea; rendered views sanitize HTML.
