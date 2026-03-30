# ANTHILL-WORKERS-UX: Task Visibility and Worker Interaction

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-WORKER, ANTHILL-DASHBOARD                            |
| Related    | ANTHILL-CHAT                                                 |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

The Workers tab provides real-time visibility into what each AI worker is
doing. Users can monitor task progress, send follow-up context to running
tasks, and cancel tasks that are no longer needed. A compact task bar in the
page header provides at-a-glance status across all tabs.

This specification defines the rendering rules, interaction patterns, and
WebSocket event mappings that an implementation MUST follow to present worker
activity in the Anthill web dashboard.

---

## 2. Task Bar (Header)

The task bar is a compact status indicator embedded in the page header. It
is visible regardless of which tab is active, giving users persistent
awareness of background work.

### 2.1 Placement

The task bar MUST be rendered inside the header element, to the right of the
tab bar. It is identified as the `#task-bar` element.

### 2.2 Content

- When there are no active tasks and no typing indicator, the task bar
  MUST be empty (no text rendered).
- When one or more tasks are running, the task bar MUST display a count
  badge in the format: `N worker(s) active`, where `N` is the number of
  active tasks for the currently selected ANT. The word "worker" MUST be
  pluralised when `N > 1`.

### 2.3 Styling

The task bar MUST use a subdued text colour (`var(--text-dim)`) at 13px
font size. It MUST NOT draw attention away from the primary content area.

### 2.4 Navigation

Clicking the task bar SHOULD switch the user to the Workers tab.

---

## 3. Task Panel (Below Chat)

A secondary tasks panel (`#tasks-panel`) MUST appear below the chat area
when tasks are active. This panel provides a compact summary without
requiring a tab switch.

### 3.1 Visibility

- The panel MUST have `display: none` by default.
- When one or more tasks are active, the panel MUST add the CSS class
  `has-tasks`, which sets `display: block`.
- When only a typing indicator is active (no tasks), the panel MUST
  display an italic "Thinking..." message in dim text.

### 3.2 Panel Constraints

The panel MUST be constrained to a maximum height of 120px with vertical
scroll overflow. It MUST have a top border (`1px solid var(--border)`) and
surface background colour (`var(--surface)`).

### 3.3 Task Items

Each task MUST be rendered as a `.task-item` containing:

1. **Preview**: `#N PREVIEW_TEXT` where `N` is the task ID and
   `PREVIEW_TEXT` is the truncated initial prompt. The preview MUST use
   `var(--text-dim)` colour, fill available space (`flex: 1`), and apply
   `text-overflow: ellipsis` with `white-space: nowrap`.

2. **Elapsed Time**: displayed in `var(--yellow)` with 12px horizontal
   margin. Format: `Ns` when under 60 seconds, `Nm Ns` otherwise (e.g.,
   `42s`, `2m 15s`).

3. **Cancel Link**: a `cancel` text link in `var(--red)` at 12px font
   size. Clicking MUST invoke the cancel mechanism defined in Section 8.

4. **Progress Text**: the latest progress detail, displayed below the
   header row at 12px font size. Colour MUST follow the colour rules
   defined in Section 6.

5. **Agent Spawns**: any progress entries of kind `agent_spawn` MUST be
   rendered as indented sub-items (`padding-left: 12px`) prefixed with the
   `\u21b3` character, in `var(--yellow)` at 11px font size.

---

## 4. Worker Cards (Workers Tab)

The Workers tab (`#workers-panel`) provides a detailed view of each running
task as a card.

### 4.1 Empty State

When no tasks are active for the selected ANT, the workers list MUST
display a centered message: "No active workers. Send a message to start
one." The text MUST use `var(--text-dim)` colour with 40px vertical
padding.

### 4.2 Card Layout

Each task MUST be rendered as a card with the following structure:

```
+-----------------------------------------------------------+
| #N  PREVIEW_TEXT                         ELAPSED  [Cancel] |
|                                                            |
| LATEST_PROGRESS_TEXT                                       |
|   -> agent_spawn_1                                         |
|   -> agent_spawn_2                                         |
|-----------------------------------------------------------|
| progress_entry_1                                           |
| progress_entry_2                                           |
| ...                                                        |
|                                                            |
| [follow-up input field                     ] [Follow-up]   |
+-----------------------------------------------------------+
```

### 4.3 Card Styling

Each card MUST use:
- Background: `var(--surface)`
- Border radius: 10px
- Padding: 20px
- Bottom margin: 14px
- Full width with `box-sizing: border-box`

### 4.4 Card Header

The header row MUST use `display: flex` with `justify-content: space-between`.
It contains:

- **Task ID**: rendered as `#N` in bold (700 weight) using
  `var(--accent)` colour at 16px font size.
- **Preview Text**: the initial prompt text, HTML-escaped, displayed after
  a 10px left margin.
- **Elapsed Time**: rendered in `var(--yellow)` at 15px font size, 600
  weight.
- **Cancel Button**: a button with no background, `var(--red)` text, a
  1px solid `var(--red)` border, 6px border radius, 6px/14px padding, at
  14px font size. Clicking MUST invoke the cancel mechanism defined in
  Section 8.

### 4.5 Progress Display

Below the header, the latest progress text MUST be displayed at 15px font
size with 6px top margin and 8px vertical padding. The text colour MUST
follow the rules in Section 6.

### 4.6 Agent Spawns

Progress entries with `kind === "agent_spawn"` MUST be collected and
rendered as indented sub-items below the progress line. Each sub-item
MUST be:
- Prefixed with the `\u21b3` character
- Styled in `var(--yellow)` at 12px font size
- Indented with `padding-left: 12px`
- Separated by 2px top margin

### 4.7 Progress History

The last 5 progress entries MUST be displayed in a history section below
the agent spawns. This section MUST:
- Have a 10px top margin
- Be separated by a top border (`1px solid var(--border)`)
- Use 10px top padding
- Render each entry at 13px (outer container) with individual entries at
  11px font size in `var(--text-dim)` colour

The client MUST retain up to 20 progress entries per task in memory. When
the count exceeds 20, the oldest entries MUST be discarded (keeping the
most recent 20). The history section displays only the last 5 of these
retained entries.

---

## 5. Progress Event Types

The server emits `task_progress` WebSocket events with a `kind` field that
determines how the progress is displayed. The following table defines
the mapping from event kind to visual presentation.

### 5.1 Event Schema

```json
{
  "type": "task_progress",
  "bot": "<ant-id>",
  "task_id": <u32>,
  "kind": "<string>",
  "detail": "<human-readable text>"
}
```

### 5.2 Kind Mapping

| Kind             | Colour                | Display Behaviour                                               |
|------------------|-----------------------|-----------------------------------------------------------------|
| `thinking`       | `var(--green)`        | Shows detail text as green progress. Generated by Gemini `thought` events and session init. |
| `tool_use`       | `var(--green)`        | Shows tool name and context: "Reading: path", "Editing: path", "Running: cmd", "Searching: pattern", "Grep: pattern", "Using: tool", "Tool: name". |
| `agent_spawn`    | `var(--yellow)`       | Rendered as an indented sub-item prefixed with `\u21b3`, not inline progress text. |
| `question`       | `#e090ff` (purple)    | Sets `needsInput` flag on the task. Triggers auto-switch to Workers tab (Section 11). Changes follow-up input placeholder (Section 7). |
| `warning`        | `var(--yellow)`       | Sets `warning` flag on the task. Stall watchdog emits this after 120 seconds of no output. Detail format: "No output for Ns -- worker may be stalled". |
| `fallback`       | `var(--green)`        | Shows which backend failed and which is being tried next. Detail format: "FAILED_BACKEND failed, trying NEXT_BACKEND...". |

### 5.3 Colour Priority

When determining the progress text colour, the implementation MUST apply
the following priority (highest first):

1. If `task.error` is true: `var(--red)`
2. If `task.needsInput` is true: `#e090ff` (purple)
3. If `task.warning` is true: `var(--yellow)`
4. Otherwise: `var(--green)`

### 5.4 Error Overlay

When a `task_error` event is received, the task's `lastProgress` field MUST
be prefixed with a warning emoji and the error text. The `error` flag MUST
be set to `true`, causing the progress colour to become `var(--red)`.

---

## 6. WebSocket Events

The following WebSocket events govern the task lifecycle in the dashboard.
All events are broadcast on the global event channel and delivered to
connected WebSocket clients as JSON with a `type` discriminator.

### 6.1 task_started

Emitted when a new AI worker task begins.

```json
{
  "type": "task_started",
  "bot": "<ant-id>",
  "task_id": <u32>,
  "preview": "<truncated prompt text>"
}
```

On receipt, the client MUST:
- Create a new task entry in `state.tasks[bot]` with the given `task_id`,
  `preview`, `startTime` set to `Date.now()`, and an empty `progress`
  array.
- Re-render the task bar and bot list.

### 6.2 task_progress

Emitted during task execution with real-time progress updates. Schema
defined in Section 5.1. On receipt, the client MUST:
- Locate the matching task by `bot` and `task_id`.
- Update `lastProgress` with the `detail` field.
- If `kind` is `"question"`, set `task.needsInput = true`.
- If `kind` is `"warning"`, set `task.warning = true`.
- Append the progress entry to the task's `progress` array.
- Trim the `progress` array to the most recent 20 entries.
- Re-render the task bar and, if the Workers tab is active, re-render
  worker cards.

### 6.3 task_completed

Emitted when a task finishes (successfully or via cancellation).

```json
{
  "type": "task_completed",
  "bot": "<ant-id>",
  "task_id": <u32>,
  "duration_secs": <u64>
}
```

On receipt, the client MUST:
- Remove the task from `state.tasks[bot]`.
- Re-render the task bar, bot list, and (if active) worker cards.

### 6.4 task_error

Emitted when a task encounters an error.

```json
{
  "type": "task_error",
  "bot": "<ant-id>",
  "task_id": <u32>,
  "error": "<error message>"
}
```

On receipt, the client MUST:
- Locate the matching task and set `lastProgress` to the error message
  prefixed with a warning indicator.
- Set the task's `error` flag to `true`.
- Re-render the task bar and worker cards.
- The task MUST NOT be removed from the display on error alone; it remains
  visible until a `task_completed` event arrives.

---

## 7. Follow-up Input

Each worker card MUST include a follow-up input mechanism allowing users
to send additional context to a running task.

### 7.1 Layout

The follow-up area MUST appear at the bottom of each worker card and
contain:
- A text input field (`<input type="text">`) with `id="followup-{task_id}"`.
- A "Follow-up" button adjacent to the input.

The input and button MUST be laid out in a flex row with 8px gap and 12px
top margin.

### 7.2 Input Styling

The input field MUST use:
- Background: `var(--bg)`
- Text colour: `var(--text)`
- Border: `1px solid var(--border-light)`, or `1px solid #e090ff` when a
  question is pending (`task.needsInput === true`)
- Border radius: 6px
- Padding: 8px 12px
- Font size: 14px
- `flex: 1` to fill available width

### 7.3 Placeholder Text

- Default: `"Add context for this task..."`
- When `task.needsInput` is true: `"Answer the question above..."`
- After successful submission: `"Queued! Will run when task finishes."` for
  3 seconds, then revert to the default placeholder.

### 7.4 Submission

The follow-up MUST be submitted when the user presses Enter or clicks the
Follow-up button. On submission:

1. The client MUST send a WebSocket command:
   ```json
   {
     "type": "followup",
     "bot": "<active-ant-id>",
     "task_id": <u32>,
     "message": "<trimmed input text>"
   }
   ```
2. The input value MUST be cleared.
3. The placeholder MUST change to `"Queued! Will run when task finishes."`
   and revert after 3000 milliseconds.

Empty or whitespace-only input MUST NOT be submitted.

### 7.5 Server-Side Handling

The server MUST enqueue the follow-up message in the per-task follow-up
queue (`handle.follow_ups`), keyed by `task_id`. The follow-up is stored
as a `FollowUp` struct with `chat_id: 0`, the message text, and source
`"web"`.

### 7.6 Input Preservation Across Re-renders

Because worker cards are re-rendered every second (Section 9) and on every
progress event, the implementation MUST preserve follow-up input state
across re-renders:

1. Before re-rendering, the implementation MUST save the current value of
   each `followup-{id}` input element into a `savedInputs` map.
2. The implementation MUST record whether a follow-up input had focus and,
   if so, capture the cursor position (`selectionStart`).
3. After re-rendering, the implementation MUST restore each saved input
   value to its corresponding element.
4. If a follow-up input had focus before re-render, the implementation
   MUST restore focus to that element and set the cursor position to the
   lesser of the saved position and the current value length.

---

## 8. Cancel

Each task MUST have a cancel control. Cancellation is available both in the
task panel (Section 3) and in worker cards (Section 4).

### 8.1 Client-Side

On cancel, the client MUST send a WebSocket command:

```json
{
  "type": "cancel",
  "bot": "<active-ant-id>",
  "task_id": <u32>
}
```

The client MUST NOT remove the task from the display on its own. Removal
happens only when the server broadcasts a `task_completed` event in
response.

### 8.2 Server-Side

On receiving a cancel command, the server MUST:

1. Look up the task in the bot's `TaskMap` by `task_id`.
2. Remove the task entry from the map.
3. Abort the task's Tokio handle (`task.handle.abort()`).
4. Broadcast a `task_completed` event with `duration_secs: 0` so that
   all connected clients remove the task from their display.
5. Broadcast a chat `message` event with text `"Cancelled task #N."` so
   the cancellation is visible in the chat history.

### 8.3 REST Endpoint

Cancellation is also available via the REST API:

```
POST /api/ants/{id}/cancel/{task_id}
```

This endpoint MUST remove the task from the bot's task map, abort the
task handle, and return `200 OK`. If the task or bot is not found, it MUST
return `404 Not Found`.

### 8.4 Process Group Termination

CLI backend subprocesses are spawned in their own process group
(`.process_group(0)`). When a task is aborted or times out, the
implementation MUST kill the entire process group via `SIGKILL`
(`libc::killpg(pid, SIGKILL)`) on Unix platforms to ensure no orphaned
child processes remain.

---

## 9. Timer Updates

Worker cards and the task panel MUST update every 1 second to keep elapsed
times current.

### 9.1 Task Panel Timer

A `setInterval` with a 1000ms period MUST call `renderTasks()` to update
elapsed time displays in the task panel.

### 9.2 Worker Cards Timer

A separate `setInterval` with a 1000ms period MUST call `renderWorkers()`
when the Workers tab is active (`currentTab === 'workers'`). When the
Workers tab is not active, the interval SHOULD still fire but MUST NOT
re-render (the guard condition prevents unnecessary DOM updates).

### 9.3 Re-render Safety

Every re-render triggered by the timer MUST preserve follow-up input
values, focus state, and cursor position as specified in Section 7.6.

---

## 10. State Synchronisation

### 10.1 Initial Load

When the WebSocket connection is established, the server sends a `hello`
message containing a `tasks` object. This object maps ANT identifiers to
arrays of active task snapshots. Each snapshot contains:

- `task_id`: the unique task identifier.
- `preview`: the truncated prompt text.
- `elapsed_secs`: seconds since the task started.
- `progress`: the latest progress text (if any).

The client MUST clear all existing task state before applying the server
snapshot, as the server is authoritative. The client MUST reconstruct
`startTime` as `Date.now() - (elapsed_secs * 1000)`.

### 10.2 Reconnection

On reconnection, the same `hello` message is sent. The client MUST wipe
stale task entries for all bots (including bots not present in the new
snapshot) before applying the fresh state. This ensures that tasks which
completed during the disconnection are not displayed as still running.

---

## 11. Auto-switch

When a `task_progress` event arrives with `kind === "question"` and the
`bot` matches the currently active ANT, the implementation MUST
automatically switch to the Workers tab by calling `switchTab('workers')`.

This ensures the user immediately sees questions that require their input,
even if they are on another tab. The auto-switch MUST NOT occur if the
question is for a different ANT than the one currently selected.

---

## 12. Conformance

An implementation claiming conformance to this specification:

1. MUST render the task bar in the header showing the active task count
   for the selected ANT (Section 2).
2. MUST render the task panel below the chat area with per-task items
   including preview, elapsed time, cancel link, progress text, and agent
   spawn sub-items (Section 3).
3. MUST render worker cards in the Workers tab with header, progress,
   agent spawns, history (last 5 entries), and follow-up input (Section 4).
4. MUST colour-code progress text according to the priority rules in
   Section 5.3.
5. MUST handle all four task lifecycle WebSocket events: `task_started`,
   `task_progress`, `task_completed`, and `task_error` (Section 6).
6. MUST preserve follow-up input values, focus, and cursor position
   across re-renders (Section 7.6).
7. MUST submit follow-up messages via the WebSocket `followup` command
   and provide appropriate placeholder feedback (Section 7).
8. MUST support task cancellation via both WebSocket command and REST
   endpoint (Section 8).
9. MUST update elapsed times every 1 second via timer intervals
   (Section 9).
10. MUST synchronise task state from the server on initial connection and
    reconnection (Section 10).
11. MUST auto-switch to the Workers tab when a `question`-type progress
    event arrives for the active ANT (Section 11).
12. MUST display the empty state message when no tasks are running
    (Section 4.1).
13. MUST retain up to 20 progress entries per task and display the last 5
    in the history section (Section 4.7).

---

## 13. References

- ANTHILL-WORKER -- AI Worker subprocess lifecycle and backend dispatch.
- ANTHILL-DASHBOARD -- Web dashboard architecture and Phoenix channels.
- ANTHILL-CHAT -- Chat interface and conversation protocol.
- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate Requirement
  Levels." IETF, 1997.
