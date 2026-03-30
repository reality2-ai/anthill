# ANTHILL-COMMS: Inter-ANT Communication

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-SENTANT, ANTHILL-COLONY                              |
| Related    | ANTHILL-KNOWLEDGE, ANTHILL-RUMINATE, ANTHILL-FEDERATION       |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

ANTs within a colony communicate through a filesystem-based inbox/outbox
protocol. The protocol is deliberately simple: an ANT writes a file to its
outbox directory; the runtime delivers that file to the target ANT's inbox
directory within 5 seconds. There is no persistent connection, no
handshake, and no acknowledgement -- delivery is fire-and-forget at the
transport layer.

Messages between ANTs are **conjectures**. Knowledge received from another
ANT MUST NOT be accepted uncritically. The receiving ANT MUST evaluate
incoming knowledge through its Popperian epistemological process, treating
it as testimony with source identifier `ant:<name>`. This is the
foundational distinction between colony communication and ordinary message
passing: every exchange is an act of Socratic discourse, not data transfer.

### 1.1 Terminology

| Term               | Definition                                                                                     |
|--------------------|------------------------------------------------------------------------------------------------|
| Colony             | A group of ANTs that share an `ants_dir` parent directory and can exchange knowledge.           |
| Inbox              | `memory/colony_inbox/` -- the directory where incoming messages arrive as JSON files.           |
| Outbox             | `memory/colony_outbox/` -- the directory where an ANT places messages for delivery.            |
| Socratic Discourse | The six-step evaluation protocol applied to every colony response (Section 5).                  |
| Loop Detection     | The `check_colony_loop` mechanism that prevents infinite back-and-forth exchanges (Section 7).  |
| Colony Tracker     | `memory/colony_tracker.json` -- persistent record of recent exchanges used for loop detection.  |
| Colony Directory   | A pre-populated listing of peer ANTs injected into the system prompt at startup.                |

---

## 2. Message Transport

Colony communication uses the local filesystem as the transport medium.
There are no sockets, no shared memory, and no message broker. The runtime
polls for new messages on a fixed interval.

### 2.1 Outbox: Sending a Message

To send a message, an ANT MUST create a file in its own
`memory/colony_outbox/` directory. The runtime reads this directory on
every poll tick and delivers any files found.

The ANT MUST ensure the directory exists before writing. The system prompt
instructs ANTs to run `mkdir -p memory/colony_outbox` as a prerequisite.

Two file formats are supported (see Section 3). After successful delivery,
the runtime MUST delete the outbox file.

### 2.2 Delivery: Outbox Processing

The `process_colony_outbox` function runs on every poll tick and performs
the following steps:

1. Read all entries in `memory/colony_outbox/`.
2. For each file, parse the target ANT name and message content according
   to the format rules in Section 3.
3. Resolve the target ANT's memory directory using `resolve_ant_memory`
   (Section 8.2).
4. If the target memory directory exists, wrap the message in a delivery
   envelope (Section 2.4) and write it to the target's `colony_inbox/`
   directory.
5. Delete the original outbox file regardless of delivery success.

Files with unrecognised formats (neither the simple format nor JSON)
MUST be skipped without deletion.

### 2.3 Inbox: Receiving a Message

The `process_colony_inbox` function runs on every poll tick and performs
the following steps:

1. Read all `.json` files in `memory/colony_inbox/`.
2. For each file, extract the `from`, `message`, and `chat_id` fields.
3. Run loop detection (Section 7). If a loop is detected, delete the
   file and skip processing.
4. If the message is non-empty and passes loop detection, dispatch it
   as a `CliRequest` with:
   - `chat_id`: `-2` (internal colony channel)
   - `source`: `"colony:<from>:<orig_chat_id>"`
   - `new_session`: `true`
5. Delete the inbox file after processing.

Only files with the `.json` extension SHALL be processed. All other files
MUST be ignored.

### 2.4 Delivery Envelope

When the runtime delivers a message from the outbox to the target's
inbox, it wraps the content in a delivery envelope. The envelope is a
JSON object written to the target's `colony_inbox/` directory with
filename `<from>-<unix_millis>.json`:

```json
{
  "from": "<sender ANT name>",
  "message": "COLONY MESSAGE from <sender>\n\n<original message>\n\n<discourse rules>",
  "chat_id": <original chat_id or 0>,
  "timestamp": "<ISO 8601 datetime>"
}
```

The `message` field is augmented with discourse rules (see Section 5.1)
that instruct the receiving ANT to advance the discussion through
Socratic dialectic with Popperian refutation.

### 2.5 Polling Interval

The colony poll interval is 5 seconds, implemented as a Tokio interval
timer with `MissedTickBehavior::Skip`. On each tick, both
`process_colony_inbox` and `process_colony_outbox` are invoked
sequentially. An implementation MUST NOT poll more frequently than once
per second. An implementation MAY use a longer interval but SHOULD NOT
exceed 10 seconds to maintain conversational responsiveness.

---

## 3. Message Format

### 3.1 Simple Format (Preferred)

The simple format is designed for ease of use by AI agents. The filename
encodes the recipient; the file content is the message body.

- **Filename**: `to-<ANT_NAME>.md` or `to-<ANT_NAME>.txt`
- **Content**: Plain text message. No JSON, no special structure.

Examples:

```
memory/colony_outbox/to-Gaea.md     → delivers to ANT "Gaea"
memory/colony_outbox/to-Alfred.md   → delivers to ANT "Alfred"
memory/colony_outbox/to-Sven.txt    → delivers to ANT "Sven"
```

The target name is extracted by stripping the `to-` prefix and the `.md`
or `.txt` suffix from the filename. Implementations MUST support both
`.md` and `.txt` extensions.

### 3.2 JSON Format (Legacy)

The JSON format provides explicit field-level control and is retained
for backward compatibility.

- **Filename**: Any `*.json` file in the outbox directory.
- **Content**: A JSON object with the following fields:

| Field     | Type   | Required | Description                              |
|-----------|--------|----------|------------------------------------------|
| `to`      | string | REQUIRED | Target ANT name.                         |
| `from`    | string | OPTIONAL | Sender ANT name (defaults to self).      |
| `message` | string | REQUIRED | Message content.                         |
| `chat_id` | number | OPTIONAL | Originating chat ID (defaults to 0).     |

### 3.3 Response Format

When the runtime forwards a colony response back to the originating ANT,
it writes a response file to the originator's `colony_inbox/` directory:

- **Filename**: `response-<ANT_NAME>-<unix_millis>.json`
- **Content**: A JSON envelope as defined in Section 2.4.

The `message` field of a response envelope contains the Socratic
Discourse prompt (Section 5) prepended to the responding ANT's actual
response text.

---

## 4. /ask Command

The `/ask` command provides a user-initiated mechanism for inter-ANT
queries, routed through the R2 event bus rather than the filesystem
outbox.

### 4.1 Syntax

```
/ask <ant-name> <question>
```

Both `<ant-name>` and `<question>` are REQUIRED. If either is missing,
the system MUST return a usage message:

> Usage: /ask \<ant-name\> \<question\>
> Example: /ask Gaea what do you know about circular economy?
>
> Or use @Name in your message to mention an ANT directly.

### 4.2 Execution Flow

1. **Context assembly.** The system calls `build_conversation_context`
   to gather the asking ANT's knowledge graph summary and recent
   conversation history.
2. **Event dispatch.** The system sends a `colony.query` event via the
   AntBus with the question and assembled context. The event uses
   `Target::Named(<target>)` routing and includes a `ReplyAddress` so
   the response can be routed back.
3. **Acknowledgement.** The system immediately returns a confirmation
   to the user:
   > Asking **@\<target\>**: _\<question\>_
   >
   > Their response will appear here when ready.
4. **Target processing.** The target ANT receives the query as a
   `CliRequest` with source `"colony:<from>:<chat_id>"` and processes
   it against its own knowledge.
5. **Response delivery.** When the target ANT completes its response:
   a. The response is displayed in the originating ANT's chat as
      `"**Response from <target>:**\n\n<text>"`.
   b. A Socratic evaluation task (Section 5) is written to the
      originating ANT's `colony_inbox/` so the originator critically
      evaluates the response.

### 4.3 ANT Not Found

If the target ANT is not found or not running, the `AntBus.ask` method
returns `false` and the system MUST return:

> ANT '\<name\>' not found or not running. Use /ants to see available ANTs.

---

## 5. Socratic Discourse Protocol

When an ANT receives a colony response, the runtime prepends a Socratic
evaluation prompt. The ANT MUST follow this six-step protocol to ensure
that knowledge exchange advances understanding rather than producing
agreement loops.

### 5.1 The Six Steps

**Step 1 -- EXAMINE.** Does this response introduce NEW knowledge or
insight? If it repeats what the ANT already knows, the ANT MUST note
that and move on. Novelty is the threshold for continued discourse.

**Step 2 -- QUESTION.** What assumptions does the other ANT make? Are
they justified? The receiving ANT SHOULD challenge weak claims with
specific counter-evidence drawn from its own knowledge graph.

**Step 3 -- CONJECTURE.** Formulate an original thesis in response.
The ANT MUST state what it thinks based on its own expertise and the
new input -- not merely echo or endorse the other ANT's position.

**Step 4 -- REFUTE.** Attempt to disprove the ANT's own thesis from
Step 3. If the thesis survives refutation, it is stronger. If it fails,
the ANT MUST say so honestly. This step embodies the Popperian principle
that conjectures earn confidence through surviving genuine refutation.

**Step 5 -- SYNTHESISE.** If both perspectives have merit, propose a
synthesis that combines the strongest elements of each. The synthesis
MUST be a new conjecture, not a compromise or averaging of positions.

**Step 6 -- ADVANCE.** End with a NEW question or direction -- not a
restatement of what has already been discussed. If the topic is
exhausted, the ANT MUST say so explicitly.

### 5.2 Discourse Rules for Outbox Delivery

When a message is delivered via the outbox (as opposed to a response
callback), the delivery envelope includes a condensed set of discourse
rules:

1. If you agree, add NEW information or a new angle.
2. If you disagree, state a clear counter-thesis with evidence.
3. If you see a synthesis, propose it and move to the next question.
4. If the topic is exhausted, say so.
5. If you have nothing new to add, say so and STOP.

### 5.3 The Silence Rule

This is the most critical rule in the protocol:

> If the ANT **agrees** with the other ANT, or the topic is exhausted,
> or the ANT has nothing new to add: the ANT MUST update its knowledge
> graph and **STOP**. The ANT MUST NOT write to `colony_outbox`. The
> ANT MUST NOT send a message saying it agrees or that the discussion
> is complete. **Silence IS the signal that the conversation has
> concluded.**

This rule exists to prevent agreement loops, where two ANTs endlessly
affirm each other's positions. An implementation MUST enforce this rule
in the system prompt. The loop detection mechanism (Section 7) provides
a safety net, but the primary enforcement is behavioural: ANTs are
instructed that silence is the correct termination signal.

---

## 6. Knowledge Integration

Knowledge received from another ANT is a conjecture. The receiving ANT
MUST integrate it into its knowledge graph using the following rules.

### 6.1 Source Identification

All knowledge originating from another ANT MUST be recorded with
`source_id` set to `ant:<name>`, where `<name>` is the sending ANT's
name. This allows the knowledge graph to track the provenance of every
claim and supports reputation tracking.

### 6.2 Evidence Classification

The receiving ANT MUST classify incoming knowledge into one of three
evidence types based on its own evaluation:

| Classification            | Condition                                        | Evidence Type             |
|---------------------------|--------------------------------------------------|---------------------------|
| Well-evidenced claim      | Consistent with the ANT's existing knowledge     | `corroboration`           |
| Contradictory claim       | Conflicts with evidence the ANT already holds     | `contradiction`           |
| Unsupported claim         | No supporting or contradicting evidence available | `inconsequential_search`  |

### 6.3 Peer Reputation

Peer reputation is itself a conjecture maintained in the ANT's
meta-graph as an `expert_in` edge linking the peer ANT to a topic.

- If the peer's response is well-evidenced and consistent with the
  receiving ANT's knowledge, the ANT SHOULD **strengthen** the
  `expert_in` edge for that peer.
- If the peer's response is weak, unsupported, or contradicts strong
  evidence, the ANT SHOULD **weaken** the `expert_in` edge.
- If the response is irrelevant or unhelpful, the ANT SHOULD NOT
  update the edge -- the exchange was inconsequential.

An ANT's reputation as an expert grows through giving good answers and
shrinks through giving bad ones -- just like any other idea in the
system.

---

## 7. Loop Detection

The `check_colony_loop` function prevents infinite or unproductive
exchanges between ANTs. It applies three independent checks to every
incoming message. If any check triggers, the message MUST be dropped
(deleted without delivery to the ANT's AI worker).

### 7.1 Exchange Cap

A hard cap of `MAX_COLONY_EXCHANGES` (currently **6**) limits the
number of messages from any single ANT within a tracking window.

- The tracker counts messages per sender ANT name.
- When the count for a given sender reaches the cap, the message is
  dropped.
- The tracker entries for that sender are cleared, allowing a fresh
  conversation to begin later.
- The cap is per ANT pair, not global. ANT A can reach its cap with
  ANT B while still accepting messages from ANT C.

### 7.2 Conclusion Detection

If the message body (case-insensitive) contains any of the following
conclusion phrases, it is dropped without delivery:

- "discussion is complete"
- "conversation is complete"
- "exchange is complete"
- "nothing new to add"
- "nothing further to add"
- "no new insights"
- "agree with your assessment"
- "agree with your conclusion"
- "we are in agreement"
- "we're in agreement"
- "topic is exhausted"
- "topic has been exhausted"
- "covered all the key"
- "covered the key points"
- "no further points"
- "no additional insights"
- "this concludes"
- "concludes our discussion"
- "thank you for the exchange"
- "thank you for this exchange"
- "productive exchange"
- "productive discussion"

The rationale is that these phrases signal the conversation has ended.
Delivering them would cause the receiving ANT to agree back, producing
an agreement loop. The Silence Rule (Section 5.3) instructs ANTs not
to send such messages, but this check provides a runtime safety net.

### 7.3 Word Overlap Detection

If the last 2 messages from the same sender ANT share more than 60% of
significant words with the current message, the message is classified
as a loop and dropped.

The algorithm:

1. Collect the last 2 messages from the same sender (from the tracker).
   If fewer than 2 previous messages exist, this check is skipped.
2. Extract significant words from the current message. A significant
   word is any whitespace-delimited token longer than 3 characters.
3. For each of the 2 previous messages, extract significant words and
   compute the intersection with the current message's word set.
4. Compute overlap ratio as `|intersection| / max(|current|, |previous|)`.
5. If **all** of the 2 previous messages exceed a 60% overlap ratio
   with the current message, classify it as a loop.

### 7.4 Colony Tracker Persistence

The colony tracker is persisted as `memory/colony_tracker.json`. It
stores a JSON array of `[sender_name, message_text]` tuples, where
`message_text` is truncated to the first 200 characters.

The tracker retains the last 20 entries. When the tracker exceeds 20
entries, the oldest entries are drained to maintain the cap.

The tracker MUST survive process restarts. Implementations MUST write
the tracker to disk after every update.

---

## 8. ANT Discovery

### 8.1 Colony Directory

At system prompt construction time, the runtime calls
`build_colony_directory` to produce a listing of all peer ANTs. This
listing is injected into the system prompt within `[COLONY]` /
`[/COLONY]` tags so the ANT knows its peers without needing to call
`list_colony_ants`.

Colony membership is determined by filesystem adjacency: all
directories under the same `ants_dir` parent that contain a
`working/memory/` subdirectory are considered colony members.

### 8.2 Memory Directory Resolution

The `resolve_ant_memory` function resolves a target ANT's memory
directory:

1. Check for `<ants_dir>/<ant_name>/ant.toml`.
2. If the file exists and contains a `[claude]` section with a
   `working_dir` field, return `<working_dir>/<memory_dir>`.
3. Otherwise, fall back to the default path:
   `<ants_dir>/<ant_name>/working/memory`.

This mechanism allows ANTs to have custom working directories while
remaining discoverable to their colony peers.

### 8.3 MCP Tool: list_colony_ants

ANTs MAY use the `list_colony_ants` MCP tool to discover peers at
runtime. However, since the colony directory is pre-populated in the
system prompt, this tool is primarily useful when the ANT needs to
refresh its peer list after a new ANT has been created.

### 8.4 MCP Tool: query_ant

ANTs MAY use the `query_ant` MCP tool to ask a peer about its area of
expertise. This is an alternative to the filesystem outbox mechanism
and is routed through the MCP plugin layer.

---

## 9. When to Communicate

The system prompt instructs ANTs on when inter-ANT communication is
appropriate. These rules are normative for conforming implementations.

### 9.1 User-Directed Communication

An ANT MUST initiate communication when the user explicitly directs it:

- "work with Gaea" -- write `memory/colony_outbox/to-Gaea.md`
- "ask Alfred about" -- write `memory/colony_outbox/to-Alfred.md`
- "check with Sven" -- write `memory/colony_outbox/to-Sven.md`
- "share this with Hine" -- write `memory/colony_outbox/to-Hine.md`
- Any mention of a colony ANT by name in any context

### 9.2 Self-Directed Communication

An ANT SHOULD initiate communication without explicit user instruction
in the following circumstances:

- The ANT encounters a topic **outside its own expertise**.
- The ANT wants to **cross-reference** its knowledge with another
  domain.
- During rumination, the ANT finds a **cross-domain pattern** that
  another ANT might have knowledge about.
- The user mentions another ANT by name in any context.

### 9.3 When NOT to Communicate

An ANT MUST NOT send a colony message solely to:

- Agree with another ANT.
- Be polite or express gratitude.
- Restate what has already been said.
- Confirm receipt of a message.

These messages produce no epistemic value and risk triggering agreement
loops. The Silence Rule (Section 5.3) and loop detection (Section 7)
exist specifically to prevent this pattern.

---

## 10. Conformance

### 10.1 Required Behaviours

A conforming implementation:

1. MUST implement filesystem-based inbox/outbox message transport as
   described in Section 2.
2. MUST support the simple message format (Section 3.1).
3. MUST support the JSON message format (Section 3.2).
4. MUST include the Socratic Discourse Protocol prompt in colony
   response delivery (Section 5).
5. MUST enforce the Silence Rule (Section 5.3) in the system prompt.
6. MUST classify incoming colony knowledge using the evidence types
   defined in Section 6.2.
7. MUST implement all three loop detection mechanisms: exchange cap,
   conclusion detection, and word overlap (Section 7).
8. MUST persist the colony tracker to disk (Section 7.4).
9. MUST implement ANT discovery via filesystem adjacency (Section 8.1).
10. MUST implement `resolve_ant_memory` with `ant.toml` fallback
    (Section 8.2).

### 10.2 Optional Behaviours

A conforming implementation:

1. MAY adjust the polling interval, but it MUST NOT be shorter than
   1 second and SHOULD NOT exceed 10 seconds.
2. MAY adjust `MAX_COLONY_EXCHANGES`, but it MUST NOT be less than 3
   and SHOULD NOT exceed 10.
3. MAY extend the conclusion phrase list (Section 7.2), but MUST NOT
   remove any of the defined phrases.
4. MAY adjust the word overlap threshold (Section 7.3), but it MUST
   remain between 0.4 and 0.8 inclusive.
5. MAY implement the `/ask` command (Section 4) via AntBus event
   routing or via the filesystem outbox mechanism.

---

## 11. References

- Popper, K. R. *The Logic of Scientific Discovery*. Routledge, 1959.
- Plato. *Meno*, *Theaetetus* -- Socratic method as philosophical inquiry.
- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate Requirement
  Levels." IETF, 1997.
- ANTHILL-SENTANT -- ANT conductor FSM and plugin architecture.
- ANTHILL-COLONY -- colony supervisor and ANT lifecycle.
- ANTHILL-KNOWLEDGE -- knowledge graph schema and persistence.
- ANTHILL-RUMINATE -- autonomous thinking and cross-domain discovery.
- ANTHILL-FEDERATION -- cross-node colony mesh.
