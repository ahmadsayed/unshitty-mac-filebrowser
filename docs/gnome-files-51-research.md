# GNOME Files 51: verified features worth considering

Research checked **2026-10-29** against primary GNOME sources only.
This is a behaviour-inspiration shortlist and source-checked comparison with
File Browser v0.5.3 (`5860279`), not an implementation plan.
All “adaptation” suggestions below are our analysis, not promises made by GNOME.

## Release certainty

- **GNOME 51 exists and is released.** The official announcement is dated
  **2026-09-16**, codename “A Coruña”; it explicitly describes Files improvements. [R51]
- **Nautilus 51.0 is tagged:** the official tag points to release commit
  `31b3161eddbe9db0a8e736d76056bc1c549474cf`, dated **2026-09-14**. [TAG]
- The official source archive publishes **51.0.1** on **2026-09-14** and
  **51.1** on **2026-10-11**; its latest marker is `LATEST-IS-51.1`. [INDEX]
- Changes first documented in **51.alpha (2026-06-29)**,
  **51.beta (2026-08-05)** and **51.rc (2026-08-31)** are retained in the
  **tagged 51.0 NEWS**. These are released-cycle changes, not merely open proposals. [INDEX] [N51]
- **Development/unreleased proposals:** none is asserted in this shortlist.
  An open MR, a design discussion, or mutable `main` alone would not prove shipment.
  Absence from this report does not mean there are no ongoing development proposals.
- Dates for prereleases and maintenance releases above are the official archive's
  file timestamps, not inferred distro availability dates. [INDEX]

## Eight supported candidates

### 1. Selection that survives context menus and delayed copy completion

**Confirmed in 51:** right-clicking folder background no longer clears selection;
copied files become selected after completion, but operation results must not
override a user's newer manual selection. First documented in 51.alpha; the first
two changes also appear in the September release notes. [ALPHA] [N51] [R51]
**Adaptation:** track a selection revision and destination/tab identity when an
operation starts. Auto-select its successful outputs only if the user has not
changed selection or navigated. Preserve selection when opening a background menu.
**Priority:** high; small visible polish with important asynchronous race tests.

### 2. A dragged-item count badge

**Confirmed in 51:** dragging multiple files displays the selected-item count on
the drag representation; documented in 51.alpha and official release notes. [ALPHA] [R51]
**Adaptation:** draw an original, themed drag image with the count of actual
dragged entries, not the number of rendered DOM rows. Test both grid and list,
off-screen selections, and the distinction between internal and native OS drags.
**Priority:** high; useful reassurance before moving a large selection.

### 3. Clear permission and symlink emblems

**Confirmed in 51:** show either unreadable or read-only status, and prioritize
the symbolic-link emblem before those statuses. The release notes highlight
correct folder states; **51.1** additionally fixes the unwritable indicator. [ALPHA] [R51] [N511]
**Adaptation:** return structured capabilities from Rust and render original
emblems with a documented precedence. Do not equate read-only with unreadable,
or assume permission mode bits alone describe ACLs, mounts or remote capabilities.
**Priority:** medium; especially useful for read-only SFTP and received shares.

### 4. Better tab handling, including narrow windows

**Confirmed in 51:** 51.beta adds a tab overview in narrow mode, opens locations
requested by other apps in new tabs, and avoids duplicate loading locations.
51.rc fixes single-tab overview behaviour; 51.1 fixes tab reuse for shell requests. [BETA] [RC] [N511]
**Adaptation:** build an original compact tab switcher in TypeScript; define when
an external open request reuses a tab versus creates one. Keep loading-location
identity separate from completed navigation to prevent duplicate requests.
**Priority:** medium; useful on small windows, with real OS integration testing.

### 5. Empty-document fallback when there are no templates

**Confirmed in 51:** 51.beta adds an empty-document menu entry when the templates
directory is empty. This is a fallback improvement, not the invention of the
longstanding template-based New Document feature. [BETA] [N51]
**Adaptation:** always provide a simple blank document route when templates are
missing; create via Rust with collision handling and select the created file.
**Priority:** small usability win; verify whether our existing New Document
already covers it before adding anything.

### 6. A cleaner, type-to-search Open With chooser

**Confirmed in 51:** duplicate app entries are suppressed in 51.beta; 51.rc adds
type-to-search and ensures each app appears in only one section. [BETA] [RC] [N51]
**Earlier baseline:** an app-chooser search entry was restored in **50.2**, so
“searchable Open With” itself must not be marketed as new in 51. [N502]
**Adaptation:** deduplicate by stable app identifier and normalize labels for
filtering; preserve keyboard selection and let Escape close the chooser.
**Priority:** medium; platform app discovery belongs behind Rust platform seams.

### 7. Preview as a lightweight action surface

**Confirmed in the 51 cycle:** Nautilus 51.rc adds rename, trash and folder-opening
functionality to the previewer integration. Separately, the official GNOME 51
notes describe **Sushi**, not Nautilus itself, receiving a GTK4/libadwaita revamp,
dark mode, real-size images and modern image/document rendering libraries. [RC] [R51]
**Adaptation:** extend our own Space preview with Open, Rename and Move to Trash
using existing commands/dialogs. Consider natural-size image display independently;
neither GTK migration nor copying Sushi UI/code/assets is required or permitted.
**Priority:** medium; safe actions are attractive, full media preview is larger work.

### 8. Smoother loading and timely search feedback

**Confirmed in 51:** 51.alpha removes several main-thread blocking I/O uses and
slightly improves reload performance; 51.rc delays clearing the old view for
smoother navigation. 51.beta moves search focus to a higher-relevance result. [ALPHA] [RC] [BETA]
**Earlier baseline:** time-based rather than file-count-based search-result
flushing is already in **50.2 (2026-05-24)**, not a 51 innovation. [N502] [INDEX50]
**Adaptation:** bounded timed result batches, cancellation/generation IDs, and
nonblocking Rust work; consider keeping the old view visible with an explicit
loading state. Never let stale displayed files act as though they belong to the
new destination, or let late relevance updates steal a user's manual selection.
**Priority:** high; measure first-result latency, slow-folder reloads and 10k entries.

## Do not relabel older features as “new in 51”

- **GNOME 50, released 2026-03-18:** multi-file-type search filters,
  case-insensitive path completion, highlighted batch-rename replacement text,
  pop-out properties and a grid-caption configuration dialog. [R50]
- **GNOME 50:** faster thumbnail/icon loading, lower memory usage, increased
  test coverage and sandboxed image decoding with Glycin are its own release
  improvements, even though responsiveness work continues in 51. [R50]
- **GNOME 46, released 2024-03-20:** global search across configured locations,
  sidebar operation progress and instantaneous grid/list switching. Its notes
  describe content search and file-type/date filtering as existing capabilities,
  not newly introduced in either 46 or 51. [R46]
- **51.1 safety divergence:** upstream NEWS says Replace becomes the default in
  the conflict dialog. **Do not adopt that:** this repository explicitly requires
  Enter to choose Keep Both. Behaviour inspiration is not blind parity. [N511]

## Recommendations for this app

The main session compared the shortlist with the actual v0.5.3 implementation.
Do not spend a new task rebuilding what is already here.

| Priority | Feature to bring over | Current gap and suggested scope |
|---|---|---|
| 1 | **Space preview actions** | `previewEntry` displays text or a 1024px thumbnail, without action buttons. Add Open, Rename and Move to Trash through the existing dialogs/commands, plus Open Containing Folder for results. Remote/read-only locations must not gain write actions. [RC] |
| 2 | **Read-only vs unreadable emblems** | `Entry` and `entryIcon` carry symlink/cloud/version information but no effective access capabilities. Fetch visible-item capability information asynchronously and show a lock/no-access emblem with tooltips. Keep OS/ACL logic in Rust/platform functions; do not infer access solely from Unix mode bits. [R51] [N511] |
| 3 | **Type-to-search Open With** | `showOpenWith` builds a full app list with no filter entry or type-to-search. `platform::applications` already deduplicates IDs, so preserve that and add filtering/keyboard focus; MIME recommendations would be an additional independently scoped improvement. [RC] |
| 4 | **Don't steal a newer selection after copy** | `transferBatch` ends with `reload(done)` when still in the destination. That can select outputs over a newer manual selection. Track selection and navigation/tab revisions to preserve the user's newer intent. Background context menus already avoid clearing selection in `onViewContextMenu`. [ALPHA] |
| 5 | **Compact tab overview** | `renderTabbar` renders tabs but no overview/overflow picker. Add a searchable or compact all-tabs popover for narrow windows/many tabs, reusing the existing tab state. External open-request tab reuse is a separate platform-integration task. [BETA] [N511] |

### Already implemented — verify, don't reimplement

- **Drag count:** `beginDrag` renders `drag-count` for multiple selected entries
  (`src/main.ts:3641`). It uses the selected-entry array, not just rendered rows.
- **Select copied items:** `transferBatch` calls `reload(done)` on the open
  destination; the missing refinement is protecting a newer manual selection.
- **Background context-menu preservation:** `onViewContextMenu` opens the
  background menu without clearing selection. Check normal and macOS Ctrl-click
  event sequences in a real app before claiming every interaction matches 51.
- **Blank documents without templates:** `showNewDocumentDialog` always offers
  Empty Text File before reading templates.
- **Responsive listing:** `navigate` retains the old folder while the next
  listing is awaited, listing work is offloaded in Rust, views are virtualized,
  and sorted listings are reused across presentation-only changes.
- **Progressive filename search:** batches, cancellation, accent/case folding
  and limit feedback are implemented. A lone early result can still wait too
  long; this is tracked by issue #39, not a new task to duplicate.

### Worth considering, but not “new Files 51 features”

- **Search filters:** Images/PDF/Documents plus Modified/Created date and scope
  would be a useful gap to close; official 50/46 notes establish the older
  origins. Current search is filename-only, without type/date controls. [R50] [R46]
- **Photo metadata in preview/properties:** GNOME 51's **Loupe**, a separate
  image viewer, adds lens/equipment, creator/copyright and embedded color-profile
  information. This could enrich our image inspection later, but is not a
  Nautilus 51 feature. [R51]
- **Undo/redo history and batch-rename numbering:** useful independent app gaps,
  not established by these sources as new additions in 51. This app's Undo is
  only the most recent Trash batch; batch rename has find/replace/prefix/suffix.
  Scope and test data-safety before broadening either.

**Recommended first task:** preview actions; it gives a visible improvement by
reusing existing commands, without changing filesystem semantics. Follow with
Open With search and copy-selection race protection. Implement fresh code and
original icon assets; none of this research authorizes copying GNOME sources.

## Boundaries and evidence quality

Only documentation was changed; no Nautilus binary was exercised and no new
features were implemented. The existing app passed TypeScript/build, 244 frontend
tests and 550 Rust tests on macOS. There is no visible change or native feature
verification to claim for this documentation task.
Implement behaviour independently in Rust/Tauri/TypeScript; copy no GNOME code,
icons or other assets, and preserve our data-safety and virtualization rules.
GitLab `webfetch` returned HTTP 406; plain `curl` successfully retrieved the tag
and tagged NEWS. The GNOME GitHub mirror's `master` NEWS stopped at 44.0 during
this check, so it was **not** used as evidence of current release state.
Prefer tagged NEWS and dated official archive notes over mutable branch content.

## Primary-source URLs

[R51]: https://release.gnome.org/51/ "GNOME 51 official release notes, 2026-09-16"
[TAG]: https://gitlab.gnome.org/GNOME/nautilus/-/tags/51.0 "Nautilus 51.0 official tag, release commit dated 2026-09-14"
[N51]: https://gitlab.gnome.org/GNOME/nautilus/-/raw/51.0/NEWS "Tagged Nautilus 51.0 NEWS; includes alpha/beta/rc history"
[INDEX]: https://download.gnome.org/sources/nautilus/51/ "Official 51 release archive and publication timestamps"
[ALPHA]: https://download.gnome.org/sources/nautilus/51/nautilus-51.alpha.news "51.alpha NEWS, archive timestamp 2026-06-29"
[BETA]: https://download.gnome.org/sources/nautilus/51/nautilus-51.beta.news "51.beta NEWS, archive timestamp 2026-08-05"
[RC]: https://download.gnome.org/sources/nautilus/51/nautilus-51.rc.news "51.rc NEWS, archive timestamp 2026-08-31"
[N511]: https://download.gnome.org/sources/nautilus/51/nautilus-51.1.news "51.1 maintenance NEWS, archive timestamp 2026-10-11"
[R50]: https://release.gnome.org/50/ "GNOME 50 official release notes, 2026-03-18"
[N502]: https://download.gnome.org/sources/nautilus/50/nautilus-50.2.news "50.2 NEWS, also inspected in tagged 51.0 NEWS"
[INDEX50]: https://download.gnome.org/sources/nautilus/50/ "Official 50 release archive; 50.2 published 2026-05-24"
[R46]: https://release.gnome.org/46/ "GNOME 46 official release notes, 2024-03-20"
