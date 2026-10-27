# Z6 Accessibility and Reduced-Motion Audit

**Date:** 2026-10-26  
**Scope:** `apps/selora-desktop/frontend/src/components/**`, `src/layout/**`, and `src/styles/*.css` except `onboarding.css`; forbidden files (`AboutDialog*`, `src/app/App.tsx`, `package.json`, backend/cargo) were not edited.  
**Requirements read:** `PHASE_Z_TAURI_PROMPT.md` §13 (lines 680-704), `DESIGN_TOKENS.md` §5/§7.4, `UI_SPEC.md` §7/§13.

## Summary

- Fixed 19 accessibility/reduced-motion defects across dialogs, focus, interactive semantics, live regions, disabled affordances, and motion.
- Verified 10 additional areas clean; recorded 7 deferred items with their scope or risk reason.
- Frontend gate passed: typecheck clean, **41 test files / 291 tests**, production build successful.

## Fixed Findings

| ID | Area | Finding | Severity | Fix / status | Evidence |
|---|---|---|---|---|---|
| Z6-A11Y-01 | Dialog: notifications | Notification popover lacked `role="dialog"`, `aria-modal`, a labelled heading, initial focus, and Tab containment. | High | **Fixed.** Added labelled dialog, `tabIndex={-1}`, initial focus on the first enabled action (or panel), Tab trap, and Escape/outside-click focus restore. | `apps/selora-desktop/frontend/src/components/NotificationCenter.tsx:103`, `:130`, `:188` |
| Z6-A11Y-02 | Dialog: command palette | Palette had only an `aria-label`, Escape was handled only by the input, and Tab skipped category pills. | High | **Fixed.** Added hidden `<h2>` + `aria-labelledby`; dialog-level Escape and a real Tab cycle across input, pills, and result options. | `apps/selora-desktop/frontend/src/components/CommandPalette.tsx:213`, `:240`, `:246` |
| Z6-A11Y-03 | Overlay: agent sessions | Session drawer was an unlabelled overlay with no focus entry/containment or restore from backdrop/close. | High | **Fixed.** Added `role="dialog"`, `aria-modal`, labelled heading, initial focus, Tab trap, and Escape/backdrop/close focus restore. | `apps/selora-desktop/frontend/src/layout/AgentDock.tsx:37`, `:39`, `:193`, `:198` |
| Z6-A11Y-04 | Interactive: motion toggle | Calm-mode toggle was a clickable `<div>`. | High | **Fixed.** Replaced with a real `<button role="switch">` with `aria-checked`, accessible name, title, and visible pressed styling. | `apps/selora-desktop/frontend/src/layout/ActivityBar.tsx:200` |
| Z6-A11Y-05 | Interactive: editor tabs | Tabs were clickable `<div role="tab">` elements with a nested close `<button>` (invalid nested interactive content). | High | **Fixed.** Real `<button role="tab">` now sits in a non-interactive shell; close is a sibling button. Roving tabindex, arrow/Home/End, and focus movement preserved. | `apps/selora-desktop/frontend/src/components/editor/EditorTabBar.tsx:106`, `:110`, `:129` |
| Z6-A11Y-06 | Interactive: minimap | Minimap navigation was pointer-only. | Medium | **Fixed.** Focusable `role="slider"` with Arrow/Page/Home/End navigation and `aria-valuemin/max/now/text`. | `apps/selora-desktop/frontend/src/components/editor/EditorMinimap.tsx:57`, `:80`, `:82` |
| Z6-A11Y-07 | Interactive: agent dock | In-feed plan toggle was a `<div>`; suggested prompts were unlabelled. | Medium | **Fixed.** Real button with `aria-expanded`/`aria-controls`; suggestions are a labelled group. | `apps/selora-desktop/frontend/src/layout/AgentDock.tsx:242`, `:244`, `:296` |
| Z6-A11Y-08 | Menu: title bar | Disabled later-slice item had no explanation; view switcher had no pressed state; menu focus/arrow navigation was incomplete. | Medium | **Fixed.** Explanatory `title`, `role="group"` + `aria-pressed`, initial menu focus, and Arrow/Home/End navigation. | `apps/selora-desktop/frontend/src/layout/TitleBar.tsx:73`, `:124`, `:274`, `:281` |
| Z6-A11Y-09 | Tabs: bottom dock | Tabs did not reference their panel. | Medium | **Fixed.** Every tab has `aria-controls="bottom-dock-panel"`; the panel is labelled by the active view. | `apps/selora-desktop/frontend/src/layout/BottomDock.tsx:52`, `:70` |
| Z6-A11Y-10 | Menu: breadcrumbs | Breadcrumb menus lacked accessible labelling/current state and did not restore focus on Escape. | Medium | **Fixed.** `aria-controls`/menu label, `aria-current`, and Escape focus restoration. | `apps/selora-desktop/frontend/src/components/editor/EditorBreadcrumbs.tsx:47`, `:89`, `:116`, `:120` |
| Z6-A11Y-11 | Live region: agent status | Agent state changes were visual-only. | Medium | **Fixed.** Polite, atomic live region on the status label; radar dot is decorative. | `apps/selora-desktop/frontend/src/layout/StatusBar.tsx:160`, `:166` |
| Z6-A11Y-12 | Live region: toasts | Parent and toast both advertised live-region behaviour; duplicate announcements were possible. | Low | **Fixed.** Parent is a labelled `region`; each toast keeps `role="status"` and no toast steals focus. | `apps/selora-desktop/frontend/src/components/ToastViewport.tsx:31`, `:40` |
| Z6-A11Y-13 | Reduced motion | `[data-reduced-motion]` neutralised loops but not transitions or smooth scrolling; several presence loops had no in-app freeze. | High | **Fixed.** Both OS and in-app paths disable transitions/scroll behaviour; beams, orbs, satellites, shimmers, and editor cursor freeze with semantic resting state retained. | `apps/selora-desktop/frontend/src/styles/base.css:232`, `:248`; `src/styles/agent.css:3967`; `src/styles/editor.css:509` |
| Z6-A11Y-14 | Focus visibility | Several controls removed outlines without a token replacement; some focus colours were literals. | High | **Fixed.** Token-derived `:focus-visible` rings, form-A border swap for bordered inputs, `--border-focus` for tree rows, palette form-A focus, and lit-line replacement for resize handles. | `apps/selora-desktop/frontend/src/styles/base.css:165`; `shell.css:264`, `:619`; `sidebar.css:1288`, `:1480`; `dock.css:226` |
| Z6-A11Y-15 | Disabled affordance | Filled disabled buttons still read as clickable. | Medium | **Fixed.** Z2 ghost treatment: transparent fill, hairline border, `--fg-disabled`, default cursor, and explanatory `title` on unavailable actions. | `apps/selora-desktop/frontend/src/styles/shell.css:714`, `:1609`; `sidebar.css:104`; `dock.css:106`; `agent.css:836`; `connect-modal.css:566` |
| Z6-A11Y-16 | Decoration: palette illustration | Empty-state SVG used raw hex/rgba colours. | Low | **Fixed.** Replaced with token-stroked classes (`--ink-rule`, `--err`); palette empty text also moved to `--fg-muted`/`--fg-faint`. | `apps/selora-desktop/frontend/src/components/CommandPalette.tsx:299`; `src/styles/shell.css:1928` |
| Z6-A11Y-17 | Decoration: icons | Decorative SVGs, badges, radar dots, and tooltips were exposed to assistive tech. | Low | **Fixed.** Added `aria-hidden="true"` / `focusable="false"` while retaining accessible names on their controls. | `apps/selora-desktop/frontend/src/components/Icon.tsx:207`; `SeloraMark.tsx:7`; `layout/ActivityBar.tsx:82`; `layout/StatusBar.tsx:42` |
| Z6-A11Y-18 | Structure: sidebar | Primary sidebar view title was plain text, not a heading. | Low | **Fixed.** View title is now an `<h2>`. | `apps/selora-desktop/frontend/src/layout/PrimarySidebar.tsx:52` |
| Z6-A11Y-19 | Decoration: shadows | Editor tab glow and session drawer used box-shadow/rgba layering. | Low | **Fixed.** Removed both shadows; active tab uses a token-height hairline, drawer scrim uses `--scrim`, and the hairline border carries layering. | `apps/selora-desktop/frontend/src/styles/editor.css:95`; `src/styles/agent.css:431`, `:439` |

## Verified Clean

| ID | Area | Check | Result | Evidence |
|---|---|---|---|---|
| Z6-CLEAN-01 | Dialog: cheat sheet | `role="dialog"`, `aria-modal`, labelled/described heading, initial focus, Escape, Tab trap, focus restore. | Clean | `apps/selora-desktop/frontend/src/components/CheatSheet.tsx:61`, `:102`, `:115` |
| Z6-CLEAN-02 | Dialog: About | Same dialog contract as cheat sheet; update feedback is live and errors use `role="alert"`. | Clean (out of write scope) | `apps/selora-desktop/frontend/src/components/AboutDialog.tsx:98`, `:163` |
| Z6-CLEAN-03 | Separators | Resize handles are focusable `role="separator"` controls with Arrow/Home/End support and a token-derived lit-line focus replacement. | Clean | `apps/selora-desktop/frontend/src/layout/ResizeHandle.tsx:106`, `:138`; `src/styles/shell.css:264` |
| Z6-CLEAN-04 | Buttons: empty state | Empty-state action is a real `<button>` with disabled/title support. | Clean | `apps/selora-desktop/frontend/src/components/EmptyState.tsx:48` |
| Z6-CLEAN-05 | Buttons: window/menu chrome | Window controls, tab close buttons, bottom-dock close, and palette search trigger all have accessible names. | Clean | `apps/selora-desktop/frontend/src/layout/TitleBar.tsx:163`; `components/editor/EditorTabBar.tsx:131`; `layout/BottomDock.tsx:61`; `layout/TitleBar.tsx:253` |
| Z6-CLEAN-06 | Status messages | File, search, git, output, and terminal status surfaces already use `role="status"` or polite live regions. | Clean | `apps/selora-desktop/frontend/src/features/sidebar/FileTreeView.tsx:323`; `SearchPanelView.tsx:258`; `GitPanelView.tsx:245`; `features/dock/OutputPanel.tsx:23`; `TerminalPanel.tsx:22` |
| Z6-CLEAN-07 | No span click targets | No `<span onClick>` interactive controls were found in `src/components/**` or `src/layout/**`. | Clean | Search pass over both trees |
| Z6-CLEAN-08 | Buttons: activity bar | Activity buttons are real buttons with `aria-pressed`, accessible names, and hidden decorative badges. | Clean | `apps/selora-desktop/frontend/src/layout/ActivityBar.tsx:68`, `:71` |
| Z6-CLEAN-09 | Buttons: agent suggestions | Suggested prompts are real buttons inside a labelled group. | Clean | `apps/selora-desktop/frontend/src/layout/AgentDock.tsx:296` |
| Z6-CLEAN-10 | Reduced-motion utilities | Shared animation utilities already had both OS and `[data-reduced-motion]` neutralisers; no new gaps found. | Clean | `apps/selora-desktop/frontend/src/styles/animations.css:288`, `:311` |

## Deferred Findings

| ID | Area | Finding | Severity | Reason deferred | Evidence |
|---|---|---|---|---|---|
| Z6-DEFER-01 | Dialog: connect | `ConnectModal` has `role="dialog"`, `aria-modal`, a labelled title, and Escape, but no explicit Tab trap/focus restore. | High | File is under `src/features/auth/**`, outside the allowed write scope for this pass. | `apps/selora-desktop/frontend/src/features/auth/ConnectModal.tsx:274`, `:407` |
| Z6-DEFER-02 | Interactive: legacy canvas | `CodeCanvas` line selection and hover-dismiss surfaces are pointer-only (`div` handlers). | Medium | Component is not imported or mounted anywhere in `src`; fixing dead code would expand scope without user-visible benefit. | `apps/selora-desktop/frontend/src/components/editor/CodeCanvas.tsx:71`, `:83`, `:163` |
| Z6-DEFER-03 | Interactive: session rows | `AgentSessionRail` renders session rows as `div role="button" tabIndex={0}` with no Enter/Space handler. | Medium | File is under `src/features/agent/components/**`, outside the allowed write scope; the new drawer dialog still traps focus and supports Escape. | `apps/selora-desktop/frontend/src/features/agent/components/AgentSessionRail.tsx:47`, `:51` |
| Z6-DEFER-04 | Token/decoration debt | Legacy styles still contain literal colours, shadows, gradients, and raw pixel values. Current counts across the nine audited stylesheets: **479 hex**, **71 `box-shadow`**, **15 gradients**, **1,799 raw `px`** occurrences. | Medium | Broad mechanical replacement across ~170 KB of legacy CSS risks visual regressions and exceeds a minimal a11y fix; current touched surfaces were tokenised or de-decorated surgically. | `src/styles/agent.css`, `connect-modal.css`, `shell.css`, `sidebar.css`, `editor.css`, `animations.css`, `dock.css`, `base.css`, `cheatsheet.css` |
| Z6-DEFER-05 | Tabs: bottom dock | All bottom-dock tabs remain in the Tab order; no roving tabindex or Arrow-key navigation. | Low | Real buttons, `aria-selected`, `aria-controls`, and the tabpanel are present; roving behaviour is a refinement, not a blocker. | `apps/selora-desktop/frontend/src/layout/BottomDock.tsx:44-70` |
| Z6-DEFER-06 | Menu: breadcrumbs | Breadcrumb menu items are real, labelled buttons reachable by Tab, but the WAI-ARIA menu Arrow-key pattern is not implemented. | Low | Tab access, labels, `aria-current`, Escape, and focus restore are correct; arrow navigation is a follow-up polish item. | `apps/selora-desktop/frontend/src/components/editor/EditorBreadcrumbs.tsx:111` |
| Z6-DEFER-07 | Inline styles | Layout components still use inline pixel/timing values (agent drawer transition, minimap bar widths, composer chrome). | Low | These are runtime geometry/visual tuning rather than missing semantics; moving them to CSS tokens should be done with screenshot verification. | `apps/selora-desktop/frontend/src/layout/AgentDock.tsx:78`, `components/editor/EditorMinimap.tsx:94` |

## Test Coverage Added

- `src/components/NotificationCenter.test.tsx`: **+2** tests (dialog labelling/initial focus, Tab trap); file now **11** tests.
- `src/components/CommandPalette.test.tsx`: **+1** Escape test and heading assertion; file now **9** tests.
- `src/components/editor/EditorMinimap.test.tsx`: new file, **2** tests.
- `src/components/ToastViewport.test.tsx`: new file, **1** test.
- `src/layout/AgentDock.test.tsx`: new file, **1** test, extended with dialog/initial-focus/Tab-trap assertions.
- `src/layout/ChromeBars.test.tsx`: extended switch and status-live-region assertions.
- `src/layout/EditorRegion.test.tsx`: extended roving-focus assertion and asserts the tab is a real `BUTTON`.

Net: **+7 tests**, from the 284-test baseline to **291**.

## Verification

Run from `apps/selora-desktop/frontend`:

| Command | Result |
|---|---|
| `npm run typecheck` | **Pass** (`tsc --noEmit`, exit 0) |
| `npm test` | **Pass**: **41 test files / 291 tests**, 10.27 s |
| `npm run build` | **Pass**: 1,136 modules transformed, built in 10.58 s (Vite emitted only the pre-existing chunk-size warning for chunks over 500 kB) |
