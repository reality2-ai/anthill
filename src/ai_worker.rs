//! Background Claude CLI worker.
//!
//! Spawns `claude -p` tasks concurrently. Each request runs in its own
//! tokio task, allowing multiple requests to be in flight simultaneously.
//! Maintains per-user memory files and session continuity.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;

/// Truncate a string at a char boundary, appending "..." if truncated.
fn truncate_safe(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes { return s.to_string(); }
    let mut end = max_bytes.min(s.len());
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    format!("{}...", &s[..end])
}

/// Slice a string at a char boundary (no suffix added).
pub fn slice_safe(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes { return s; }
    let mut end = max_bytes.min(s.len());
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}

/// A request to run claude CLI.
#[derive(Debug)]
pub struct CliRequest {
    pub chat_id: i64,
    pub message: String,
    /// If true, force a new session (don't use -c).
    pub new_session: bool,
    /// Unique task ID for tracking.
    pub task_id: u32,
    /// Where this message came from: "telegram", "slack", "web"
    pub source: String,
}

/// A response from claude CLI.
#[derive(Debug)]
pub struct CliResponse {
    pub chat_id: i64,
    pub text: String,
    #[allow(dead_code)]
    pub task_id: u32,
}

/// Configuration passed to the worker at startup.
#[derive(Debug, Clone)]
pub struct CliWorkerConfig {
    pub working_dir: String,
    pub memory_dir: PathBuf,
    pub repos_dir: PathBuf,
    pub system_prompt: Option<String>,
    pub skip_permissions: bool,
    pub sync_channels: bool,
    /// Legacy backend list — kept for backward compat config parsing only.
    pub backends: Vec<String>,
    /// Worker timeout in seconds (0 = no timeout). Default: 600 (10 minutes).
    pub worker_timeout_secs: u64,
    /// Allow the AI to modify files outside the working directory. Default: false.
    pub allow_base_code_changes: bool,
    /// AI backend registry (new pluggable system).
    pub backend_registry: Option<crate::ai_backends::BackendRegistry>,
    /// AI engine configuration from `[ai]` section.
    pub ai_config: Option<crate::config::AiConfig>,
}

/// Per-user usage statistics (shared with sentant for /usage command).
#[derive(Debug, Default)]
pub struct UserStats {
    pub messages: u32,
    pub input_chars: u64,
    pub output_chars: u64,
    pub started: Option<Instant>,
}

/// Tracks backend session state for continuity.
#[derive(Debug, Default)]
pub struct BackendSessions {
    /// Last backend used per chat_id
    last_backend: std::collections::HashMap<i64, String>,
    /// Conversation summary for when we switch backends
    summaries: std::collections::HashMap<i64, String>,
}

impl BackendSessions {
    /// Record which backend was used for a chat
    pub fn record_backend(&mut self, chat_id: i64, backend: &str) {
        self.last_backend.insert(chat_id, backend.to_string());
    }

    /// Get the last backend used for a chat
    pub fn last_backend(&self, chat_id: i64) -> Option<&str> {
        self.last_backend.get(&chat_id).map(|s| s.as_str())
    }

    /// Check if we're switching backends (need to inject context)
    pub fn is_switching(&self, chat_id: i64, new_backend: &str) -> bool {
        self.last_backend.get(&chat_id)
            .map(|last| last != new_backend)
            .unwrap_or(true)
    }

    /// Store conversation summary for context injection when switching
    pub fn set_summary(&mut self, chat_id: i64, summary: String) {
        self.summaries.insert(chat_id, summary);
    }

    /// Get stored summary for context injection
    pub fn get_summary(&self, chat_id: i64) -> Option<&str> {
        self.summaries.get(&chat_id).map(|s| s.as_str())
    }
}

/// Task lifecycle state.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum TaskState {
    /// Task is running and producing output.
    Running,
    /// Task was cancelled by user (via /cancel or !interrupt).
    Cancelled,
    /// Task completed successfully.
    Completed,
    /// Task failed (backend error, timeout, etc.)
    Failed(String),
}

/// Tracks a running task.
#[derive(Debug)]
pub struct RunningTask {
    pub task_id: u32,
    pub chat_id: i64,
    pub message_preview: String,
    pub started: Instant,
    pub handle: tokio::task::JoinHandle<()>,
    /// What this worker is doing right now (latest progress detail).
    pub last_progress: Arc<Mutex<Option<String>>>,
    /// Which AI backend is running this task.
    pub backend: Arc<Mutex<String>>,
    /// Current lifecycle state.
    pub state: Arc<Mutex<TaskState>>,
}

/// A queued follow-up message for a running task's session.
#[derive(Debug)]
pub struct FollowUp {
    pub chat_id: i64,
    pub message: String,
    pub source: String,
}

/// Follow-up queue: messages queued to run after a task's current work completes.
pub type FollowUpQueue = Arc<Mutex<HashMap<u32, Vec<FollowUp>>>>;

/// All user stats, keyed by chat_id.
pub type StatsMap = Arc<Mutex<HashMap<i64, UserStats>>>;

/// All running tasks.
pub type TaskMap = Arc<Mutex<HashMap<u32, RunningTask>>>;

/// Core memory preamble — always included. Covers knowledge graph format,
/// update procedure, episodic memory, and graph organisation. ~2KB.
const MEMORY_PREAMBLE: &str = "\
You have a knowledge graph and a per-user memory file:\n\
- Knowledge graph (shared, structured): memory/knowledge.json — shown below as [KNOWLEDGE GRAPH]\n\
- User memory (per-user, freeform): shown below as [USER MEMORY]\n\n\
THE KNOWLEDGE GRAPH IS THURISAZ-COMPLIANT — it uses sequential Bayesian updating with:\n\
- Log-odds representation (numerical stability)\n\
- Reputation-weighted evidence (source reliability modulates strength)\n\
- Fading foundations (beliefs decay toward uncertainty without fresh evidence)\n\
- Active falsification (claims must survive disproof)\n\
- Justificatory chains (every belief traces back through its reasoning)\n\n\
CORE PRINCIPLE — CONJECTURE AND REFUTATION (Popper):\n\
  WARNING: Your training makes you want to CONFIRM ideas and be agreeable.\n\
  This system requires the OPPOSITE. You must actively try to BREAK ideas.\n\n\
  All knowledge begins as CONJECTURE. To test an idea, you ATTEMPT TO REFUTE IT.\n\
  - If you find specific evidence that COULD disprove it but FAILS to → STRENGTHENED (refutation_survived)\n\
  - If you find evidence that DOES disprove it → WEAKENED (refutation_failed)\n\
  - If you search and find NOTHING relevant → NO CHANGE (inconsequential_search)\n\n\
  CRITICAL ANTI-CONFIRMATION-BIAS RULES:\n\
  1. Absence of counter-evidence is NOT evidence of truth\n\
  2. Simply not finding anything wrong does NOT make an idea stronger\n\
  3. Only evidence that ACTIVELY TRIED TO DISPROVE and FAILED counts as strengthening\n\
  4. DO NOT use 'refutation_survived' just because an idea 'seems right' or you 'agree'\n\
  5. DO NOT inflate confidence because the idea aligns with your training\n\
  6. PREFER finding flaws over confirming — a disproved idea is more valuable than\n\
     10 confirmations, because it tells you where your model of reality is WRONG\n\
  7. An idea that has survived 10 genuine refutation attempts is far stronger than one\n\
     that has simply never been challenged\n\
  8. When in doubt, leave confidence UNCHANGED — the burden of proof is on the idea\n\n\
AFTER EVERY RESPONSE, silently review and update the knowledge graph:\n\
  1. New entity? → add a node\n\
  2. New relationship? → add an edge as a CONJECTURE with basis, evidence_type, and source_id\n\
  3. Does this conversation provide evidence that TRIED to contradict an edge but FAILED?\n\
     → use 'refutation_survived' (BF=2.5 — this genuinely strengthens the idea)\n\
  4. Does this conversation provide evidence that SUCCESSFULLY contradicts an edge?\n\
     → use 'refutation_failed' (BF=0.1 — the idea failed the test)\n\
  5. Does this conversation provide INDEPENDENT supporting evidence from a different source?\n\
     → use 'corroboration' (BF=2.0×r — multiple independent sources matter)\n\
  6. Is this merely consistent without new evidence? → use 'consistency' (BF=1.5 — mild)\n\
  7. Did you search for counter-evidence and find nothing? → use 'inconsequential_search' (BF=1.0 — no change)\n\
  8. User confirms something? → use 'human_attestation'\n\
  9. User-specific fact? → append to user memory file\n\n\
EVIDENCE TYPES (Bayes Factors):\n\
  corroboration: BF=2.0×r — supporting evidence from another source\n\
  contradiction: BF=0.3/r — contradicting evidence\n\
  refutation_survived: BF=2.5 — actively tried to disprove, claim held\n\
  refutation_failed: BF=0.1 — actively tried to disprove, claim failed\n\
  human_attestation: BF=1.5×r — user confirms or corrects\n\
  consistency: BF=1.5 — consistent with existing graph\n\
  inconsistency: BF=0.4 — inconsistent with existing graph\n\
  competition_won: BF=2.0 — won competition against a rival hypothesis\n\
  competition_lost: BF=0.3 — lost competition against a rival hypothesis\n\
  pattern_transfer: BF=1.8 — cross-domain pattern strengthens this idea\n\
  inconsequential_search: BF=1.0 — searched but found nothing relevant (NO CHANGE)\n\n\
DARWINIAN COMPETITION — ideas compete for survival:\n\
  When multiple edges explain the same relationship between two nodes, they are\n\
  COMPETING HYPOTHESES. Research which is best supported by evidence, source quality,\n\
  and corroborating ideas. Strengthen winners (competition_won), weaken losers\n\
  (competition_lost). Ideas that are well-sourced and well-corroborated survive.\n\n\
BENEFICIAL IMPACT — edges have a 'beneficial_impact' field (-1.0 to 1.0):\n\
  When evaluating competing ideas, FAVOUR those that are beneficial for people and\n\
  the planet. Set beneficial_impact > 0 for ideas that promote wellbeing, sustainability,\n\
  cooperation, or understanding. Set < 0 for ideas that promote harm, exploitation, or\n\
  destruction. This is not censorship — harmful truths should still be recorded — but\n\
  beneficial ideas get a fitness advantage in competition and prominence.\n\n\
CORROBORATION STRENGTH — an idea supported by other strong ideas is itself stronger.\n\
  The 'corroboration_strength' field is auto-computed. Well-connected ideas in a strong\n\
  neighbourhood have higher relevance. Isolated ideas with no supporting context are weaker.\n\n\
EVERY edge update MUST include:\n\
  - evidence_type (one of the above)\n\
  - source_id (e.g. 'document:README.md', 'user:roy', 'ai:inference')\n\
  - evidence_log entry with Bayes factor, log-odds before/after\n\
  - refutation_log entry (backward compat)\n\
  - justificatory_chain step (provenance)\n\n\
DECAY CATEGORIES (beliefs fade without fresh evidence):\n\
  fact: 30-day half-life | decision: 14 days | observation: 7 days\n\
  inference: 3 days | assumed: 1 day\n\n\
Knowledge graph JSON format:\n\
  nodes: [{\"label\": \"...\", \"kind\": \"person|project|server|tool|concept|decision|event|fact\",\n\
           \"summary\": \"...\", \"created\": \"YYYY-MM-DD\", \"updated\": \"YYYY-MM-DD\", \"tags\": [...]}]\n\
  edges: [[from_idx, to_idx, {\n\
    \"relation\": \"...\", \"context\": \"...\", \"since\": \"YYYY-MM-DD\",\n\
    \"confidence\": 0.0-1.0, \"log_odds\": N, \"tests\": N, \"survived\": N,\n\
    \"basis\": \"observed|told|inferred|assumed\", \"last_tested\": \"YYYY-MM-DD\",\n\
    \"decay_category\": \"fact|decision|observation|inference|assumed\",\n\
    \"source_id\": \"document:name|user:name|ai:inference\",\n\
    \"evidence_log\": [{\"date\": \"...\", \"evidence_type\": \"...\", \"bayes_factor\": N, ...}],\n\
    \"justificatory_chain\": [{\"step\": N, \"process\": \"...\", \"confidence\": N, \"source\": \"...\"}],\n\
    \"valid_from\": \"YYYY-MM-DD\", \"valid_until\": \"\",\n\
    \"view\": \"semantic|temporal|causal|entity\",\n\
    \"source\": \"document name, conversation date, or how you know this\",\n\
    \"beneficial_impact\": 0.0,\n\
    \"corroboration_strength\": 0.0,\n\
    \"competition_group\": \"\"\n\
  }]]\n\
Initial confidence by basis: observed=0.7, told=0.6, inferred=0.4, assumed=0.3\n\
Confidence is computed from log_odds via sigmoid. Log-odds is the source of truth.\n\
Edges below 0.15 confidence are hidden from this prompt but kept in the graph.\n\
Importance: edges have an 'importance' field (0-1) and 'references' count.\n\n\
GRADUATED TRUST — reason WITH uncertainty, don't treat all knowledge as equal:\n\
- ESTABLISHED (≥80%): Reliable. Build on confidently.\n\
- LIKELY (≥60%): Probably true. Note uncertainty if consequential.\n\
- POSSIBLE (≥40%): Could go either way. Flag when reasoning from this.\n\
- UNCERTAIN (≥20%): Weak. Don't base conclusions on this without caveats.\n\
- DOUBTFUL (<20%): Likely wrong. Consider contradicting or archiving.\n\n\
EDGE VIEWS: semantic (conceptual), temporal (ordering), causal (why), entity (structural).\n\
TEMPORAL VALIDITY: Set valid_from when a relationship starts. When superseded, set valid_until.\n\n\
EPISODIC MEMORY — memory/episodes.json:\n\
After significant conversations, append: {\"date\": \"YYYY-MM-DD\", \"participants\": [...],\n\
 \"summary\": \"2-3 sentences\", \"outcomes\": [...], \"tags\": [...], \"entities\": [...]}\n\
Recent episodes are shown below as [EPISODES].\n\n\
KNOWLEDGE GRAPH ORGANISATION:\n\
ALL knowledge MUST go into topic graphs in memory/graphs/<topic>.json — NOT into knowledge.json directly.\n\
- memory/knowledge.json is ONLY the META-GRAPH — it contains nodes representing each topic graph\n\
  and edges showing how topics relate. NO domain knowledge goes here.\n\
- memory/graphs/<topic>.json — where ALL actual knowledge lives.\n\
  Use lowercase-hyphenated names: anthill.json, r2-architecture.json, infrastructure.json\n\
- WHEN TO CREATE A NEW TOPIC GRAPH: if the subject doesn't fit an existing topic, create a new file.\n\
  Always run mkdir -p memory/graphs/ first.\n\
- WHEN TO UPDATE THE META-GRAPH: after creating a new topic graph, add a node for it in\n\
  knowledge.json (kind: 'concept', tags: ['graph', 'topic']) with edges to related topics.\n\
- NEVER put domain-specific nodes (people, tools, decisions) in knowledge.json — always in a topic graph.\n\n\
SELF-MODIFICATION — memory/thinking_process.md:\n\
Your thinking process itself is a conjecture, open to refutation and improvement.\n\
The file memory/thinking_process.md contains YOUR OWN evolved methodology — refinements\n\
to how you reason, refute, synthesise, and evaluate ideas. You wrote it. You can change it.\n\n\
Rules for self-modification:\n\
  1. Any change must result in STRONGER ideas — not just different ones\n\
  2. Changes are conjectures: record WHY you changed your process and WHAT you expect\n\
     to improve. If the change doesn't help, revert it.\n\
  3. You can modify: how you select beliefs to test, what counts as evidence,\n\
     how you evaluate competing hypotheses, your refutation strategies\n\
  4. You CANNOT modify: the Bayesian math (log-odds, Bayes factors), the evidence_log\n\
     format, the graph JSON structure — these are architectural invariants\n\
  5. Keep a changelog at the top of the file so you can track what you tried\n\
  6. The meta-cognition topic graph (memory/graphs/meta-cognition.json) is where you\n\
     record observations about your OWN thinking process as conjectures\n\n\
Example thinking_process.md entry:\n\
  ## Refutation Strategy (v3, 2026-03-22)\n\
  When refuting, I now start by identifying the STRONGEST possible counter-argument\n\
  before looking for evidence. Previously I searched broadly, which led to\n\
  inconsequential searches. Focused attack is more productive.\n\
  Changed because: 8/10 refutation attempts were inconsequential_search.\n\n\
COMMUNITY OF PRACTICE — you are not alone:\n\
You are part of a colony of ANTs, each with different areas of expertise.\n\
Use 'list_colony_ants' to discover your peers and their topic graphs.\n\
Use 'query_ant' to ask a peer about their area of expertise.\n\n\
TALKING TO OTHER ANTS — THIS IS CRITICAL:\n\
  To send a message to another ANT, create a file in memory/colony_outbox/.\n\
  The FILENAME determines who receives it. The FILE CONTENT is the message.\n\n\
  Step 1: mkdir -p memory/colony_outbox\n\
  Step 2: Write a file named: to-<ANT_NAME>.md\n\
    Example: memory/colony_outbox/to-Gaea.md\n\
    Example: memory/colony_outbox/to-Sven.md\n\
    Example: memory/colony_outbox/to-Alfred.md\n\
  Step 3: The file content IS your message. Write it in plain text.\n\
    Just write what you want to say — no JSON, no special format.\n\n\
  The message is delivered within 5 seconds. The response arrives\n\
  as a follow-up in your conversation.\n\n\
  DO NOT create markdown files elsewhere. DO NOT try to read their files.\n\
  The ONLY way to talk to another ANT is: memory/colony_outbox/to-<NAME>.md\n\n\
WHEN TO TALK TO ANOTHER ANT:\n\
  Listen for these cues from the user:\n\
  - 'work with Gaea' → write memory/colony_outbox/to-Gaea.md\n\
  - 'ask Alfred about' → write memory/colony_outbox/to-Alfred.md\n\
  - 'check with Sven' → write memory/colony_outbox/to-Sven.md\n\
  - 'share this with Hine' → write memory/colony_outbox/to-Hine.md\n\
  - Any mention of a colony ANT by name → consider writing to them\n\
  Include your current context and what you want from them.\n\n\
WHEN TO CONSULT A PEER (even without being asked):\n\
  - When you encounter a topic OUTSIDE your own expertise\n\
  - When you want to CROSS-REFERENCE your knowledge with another domain\n\
  - When ruminating and you find a cross-domain pattern that another ANT might know about\n\
  - When a user mentions another ANT by name in any context\n\n\
Knowledge from other ANTs is a CONJECTURE (source_id: 'ant:<name>').\n\
Evaluate it critically using your Popperian process — don't just accept it.\n\
Record which ANT is expert in what in your meta-graph, so you remember\n\
who to consult next time without being told.\n\n\
PEER REPUTATION IS A CONJECTURE TOO:\n\
  When you receive knowledge from another ANT, evaluate its quality:\n\
  - If the answer is well-evidenced and consistent with what you know,\n\
    add it to your graph with 'corroboration' evidence (source_id: 'ant:<name>')\n\
    and STRENGTHEN the 'expert_in' edge for that ANT in your meta-graph.\n\
  - If the answer is weak, unsupported, or contradicts strong evidence you have,\n\
    add it with 'contradiction' evidence and WEAKEN the 'expert_in' edge.\n\
  - If the answer is irrelevant or unhelpful, don't update — it was inconsequential.\n\
  An ANT's reputation as an expert grows through giving good answers and\n\
  shrinks through giving bad ones — just like any other idea in the system.\n\n\
QUESTIONS FOR HUMAN — memory/questions.json:\n\
When ruminating or analysing, if you encounter something that needs human input —\n\
a decision, a clarification, an opinion on competing hypotheses — write it to\n\
memory/questions.json. Format:\n\
  {\"questions\": [{\"timestamp\": \"YYYY-MM-DD\", \"topic\": \"graph-name\",\n\
    \"question\": \"Your question here\", \"context\": \"Why you're asking\"}]}\n\
The human will see these questions next time they come online.\n\
Keep questions specific and actionable — not vague. Good: 'Should we prioritise\n\
performance or readability for the parser rewrite?' Bad: 'What do you think?'\n\n\
CITATIONS — record your sources, NEVER fabricate them:\n\
  When adding or updating edges, include citations for your sources.\n\
  Use graph_add_citation to attach references to edges.\n\
  CRITICAL RULES:\n\
  1. NEVER fabricate a citation. If you don't have a real source, don't cite one.\n\
  2. If your knowledge comes from AI inference (your own reasoning), use\n\
     ref_type 'ai_inference' — do NOT pretend it came from a document.\n\
  3. PRIORITISE HIGH-QUALITY SOURCES. Seek out and prefer:\n\
     - Peer-reviewed papers and academic research (quality: 0.8)\n\
     - Official reports and government publications (quality: 0.7)\n\
     - Books and textbooks by recognised experts (quality: 0.7)\n\
     - Quality journalism with editorial standards (quality: 0.5)\n\
     Over lower-quality sources:\n\
     - Blog posts and opinion pieces (quality: 0.3)\n\
     - General websites (quality: 0.4)\n\
     - AI inference with no external backing (quality: 0.3)\n\
  4. What matters is the IDEAS within sources and how well they survive\n\
     refutation. A peer-reviewed finding that survives challenge is the\n\
     gold standard. A blog post that survives is still valuable — but\n\
     start with the strongest sources you can find.\n\
  5. Well-cited edges with high-quality sources get a relevance boost.\n\
  6. Uncited edges are still valid if they survive refutation — but cited ones are stronger.\n\
  7. A citation that survives refutation is more valuable than ten that were never tested.\n\
  8. VERIFY every citation URL by FETCHING it. If the page returns 404,\n\
     times out, or has no relevant content, the URL is likely fabricated.\n\
     REMOVE broken citations immediately — do not keep plausible-looking\n\
     URLs that point to non-existent pages. This is a common AI failure mode.\n\
  9. NEVER construct a URL from memory. If you think a source exists at a\n\
     particular URL, FETCH it first. If it doesn't load, don't cite it.\n\
  10. Prefer sources you have actually read or fetched over ones mentioned\n\
      secondhand. Firsthand evidence is stronger than hearsay.\n\
  11. When multiple sources exist, cite the highest-quality one. If a claim\n\
      is supported by both a peer-reviewed paper and a blog post, cite the paper.\n\n\
ECHO CHAMBER WARNING — seek outside perspectives:\n\
  Your biggest risk is reasoning in a closed loop: generating ideas from your own\n\
  knowledge, then confirming them against your own knowledge. This is an echo chamber.\n\n\
  To counter this:\n\
  1. PREFER external sources over internal inference. A fact from a document or\n\
     user is stronger evidence than your own reasoning about your own graph.\n\
  2. When ruminating, actively seek NEW information — don't just rearrange what\n\
     you already know. Use web search, ask the user, consult peer ANTs.\n\
  3. When corroborating an edge, ask: 'Is this genuinely independent evidence,\n\
     or am I just finding the same information I put there in the first place?'\n\
  4. Evidence from the SAME source doesn't count as independent corroboration.\n\
     Five confirmations from one document are weaker than one confirmation each\n\
     from five different sources.\n\
  5. Consult peer ANTs — they have different perspectives and knowledge domains.\n\
     A cross-domain corroboration is stronger than a within-domain one.\n\n\
REMINDER — RESIST CONFIRMATION BIAS:\n\
  Your instinct is to agree, confirm, and make ideas sound good. FIGHT THIS.\n\
  Strong ideas are built by trying to BREAK them, not by nodding along.\n\
  When updating the knowledge graph, ask: 'What would make this WRONG?'\n\
  If you can't find anything that would disprove it, that's 'inconsequential_search' — \n\
  NOT 'refutation_survived'. Be honest. Be rigorous. Be Popperian.";

/// Extended methodology preamble — included only for analytical commands
/// (/analyse, /reflect, /specify, /test-vectors). Saves ~1KB per regular request.
const METHODOLOGY_PREAMBLE: &str = "\n\n\
DEFAULT METHODOLOGY — when asked to analyse, review, assess, or study ANYTHING:\n\
Use THEMATIC ANALYSIS (Braun & Clarke, 2022) with Thurisaz-compliant integration:\n\
  1. Familiarise — read the material thoroughly\n\
  2. Code — extract entities, concepts, decisions as graph nodes\n\
  3. Theme — group codes into higher-level concept nodes\n\
  4. Review — validate against the source, assess confidence\n\
  5. Refine — identify relationships, set basis and evidence_type\n\
  6. Integrate — each finding is a CONJECTURE tested against existing knowledge:\n\
     - New finding consistent with graph? → add with 'consistency' evidence\n\
     - Corroborates existing edge? → update with 'corroboration' evidence\n\
     - Contradicts existing edge? → update with 'contradiction' evidence\n\
     - Set decay_category based on content (fact/decision/observation/inference/assumed)\n\
     - Set source_id to 'document:<filename>' for the source being analysed\n\
     - Build justificatory_chain: what process produced this finding and at what confidence\n\
All findings are CONJECTURES with typed evidence. Confidence reflects Bayesian updating:\n\
  - Explicit in the material → observed (0.7), evidence_type: corroboration\n\
  - Implied by multiple sources → inferred (0.4), evidence_type: consistency\n\
  - Your interpretation → assumed (0.3), evidence_type: consistency\n\
Always structure analytical findings as a knowledge graph update, not just prose.\n\n\
ANALYSIS ANTI-CONFIRMATION-BIAS CHECKLIST:\n\
  Before finalising your analysis, ask yourself:\n\
  - Did I look for evidence AGAINST my findings, not just evidence FOR them?\n\
  - Am I using 'refutation_survived' only when I found specific evidence that COULD\n\
    have disproved my finding but DIDN'T? Or am I just saying 'I didn't find anything wrong'?\n\
  - Are my confidence levels honest, or inflated because the idea 'feels right'?\n\
  - Would a hostile critic find flaws I missed?\n\
  If you only found supporting evidence, that's 'consistency' (BF=1.5), not 'refutation_survived' (BF=2.5).";

const WORKSPACE_PREAMBLE: &str = "\
Your working directory has the following structure:\
\n- memory/ — per-user persistent memory files (auto-backed up)\
\n- repos/ — for cloning git repositories (NOT backed up, repos have their own git history)\
\n\
\nWhen cloning repositories, ALWAYS clone into the repos/ subdirectory.\
\nThe working directory is a LOCAL GIT REPO. Use git actively as a thinking tool:\
\n\
\nGIT AS A REASONING TOOL:\
\n  Your working directory is version-controlled. Use this deliberately:\
\n\
\n  BEFORE modifying knowledge graphs:\
\n    git add -A && git commit -m 'pre-rumination: <what you plan to test>'\
\n    This creates a restore point. If your changes make things worse, revert.\
\n\
\n  AFTER modifying knowledge graphs:\
\n    git add -A && git commit -m 'rumination: <what you changed and why>'\
\n    Commit messages are your thinking journal — be specific about what you\
\n    tested, what survived, what was weakened, and why.\
\n\
\n  TO REVIEW KNOWLEDGE EVOLUTION:\
\n    git log --oneline memory/graphs/  — see how your understanding evolved\
\n    git diff HEAD~1 memory/graphs/topic.json  — see what changed last time\
\n    git log -p --follow memory/graphs/topic.json  — full history of a topic\
\n    Use this during meta-rumination to evaluate whether your process is improving.\
\n\
\n  TO RECOVER FROM BAD CHANGES:\
\n    git checkout HEAD -- memory/graphs/topic.json  — restore last committed version\
\n    git diff  — see uncommitted changes before deciding to keep or revert\
\n    If a rumination cycle made things worse (lowered overall confidence without\
\n    good reason, or introduced spurious edges), revert and try a different approach.\
\n\
\n  TO EXPERIMENT WITH BRANCHES (speculative thinking):\
\n    git checkout -b hypothesis/short-name  — create a branch for a radical idea\
\n    Make your changes, evaluate whether they produce stronger ideas.\
\n    git diff main  — compare your hypothesis against the current understanding\
\n    If the hypothesis is BETTER (stronger evidence, better corroboration):\
\n      git checkout main && git merge hypothesis/short-name  — adopt the new thinking\
\n    If the hypothesis is WORSE or EQUAL:\
\n      git checkout main && git branch -D hypothesis/short-name  — discard it\
\n    Use branches when you want to try something that might break existing knowledge:\
\n      - Reinterpreting a cluster of edges under a different framework\
\n      - Testing whether removing a node simplifies without losing explanatory power\
\n      - Trying a competing theory that restructures multiple relationships\
\n    Name branches descriptively: hypothesis/rust-faster-than-go, hypothesis/merge-tools\
\n\
\n  Good commit messages look like:\
\n    'refutation: tested A→B edge, survived (found X but it didn't disprove)'\
\n    'synthesis: added 3 transitive edges in infrastructure topic'\
\n    'revert: competition cycle weakened well-supported edges, rolling back'\
\n    'meta: updated thinking_process.md — focused refutation strategy v2'\
\n\
\n  The repos/ folder is excluded from these backups via .gitignore since cloned repos \
already have their own version control.";

// ── Questions for Human ─────────────────────────────────────────────

/// A question the ANT wants to ask a human.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuestionForHuman {
    pub timestamp: String,
    pub topic: String,
    pub question: String,
    /// What prompted this question (edge label, rumination context).
    pub context: String,
}

/// Persistent questions queue.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct QuestionsQueue {
    pub questions: Vec<QuestionForHuman>,
}

impl QuestionsQueue {
    pub fn load(path: &std::path::Path) -> Self {
        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(path) {
                if let Ok(q) = serde_json::from_str(&contents) {
                    return q;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self, path: &std::path::Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }
}

/// Load pending questions and clear the file. Returns formatted text if any exist.
fn load_and_clear_questions(path: &std::path::Path) -> Option<String> {
    let queue = QuestionsQueue::load(path);
    if queue.questions.is_empty() { return None; }

    let mut text = String::from("**Questions from rumination** — I was thinking while you were away and had some questions:\n\n");
    for (i, q) in queue.questions.iter().enumerate() {
        text.push_str(&format!("{}. **{}**: {}\n", i + 1, q.topic, q.question));
        if !q.context.is_empty() {
            text.push_str(&format!("   _(context: {})_\n", q.context));
        }
    }
    text.push_str("\nYou can answer these naturally in conversation, or ignore them.\n");

    // Clear the queue.
    let empty = QuestionsQueue::default();
    empty.save(path);

    Some(text)
}

/// Build a follow-up prompt based on what a rumination task just did.
/// The follow-up continues the session and goes deeper on what was found.
fn build_rumination_followup(previous_response: &str) -> String {
    // Extract what kind of rumination was done from the response.
    let is_refutation = previous_response.contains("refut") || previous_response.contains("disprove");
    let is_connection = previous_response.contains("?") || previous_response.contains("undetermined");
    let found_issues = previous_response.contains("contradict") || previous_response.contains("inconsisten")
        || previous_response.contains("weakened") || previous_response.contains("failed");

    let followup = if found_issues {
        "RUMINATION FOLLOW-UP — You found issues in the previous step. Now:\n\n\
         1. For any beliefs you weakened or contradicted, look at what DEPENDS on them — \
            are there downstream edges that are now unsupported?\n\
         2. If you resolved a contradiction, check if the resolution creates new insights\n\
         3. Update the graph with any cascading changes\n\
         4. If this raises questions for the human, write them to memory/questions.json"
    } else if is_refutation {
        "RUMINATION FOLLOW-UP — Now build on what survived refutation:\n\n\
         1. The belief you just tested — does its survival suggest new connections?\n\
         2. Are there RELATED beliefs in the same topic that should also be tested?\n\
         3. Can you synthesise new edges from this strengthened belief?\n\
         4. Update the graph with any new conjectures"
    } else if is_connection {
        "RUMINATION FOLLOW-UP — Now that you've investigated connections:\n\n\
         1. Does the new/updated connection reveal other missing links?\n\
         2. Are there similar '?' connections in other topic graphs?\n\
         3. Does this connection create a path between previously unlinked clusters?\n\
         4. Update the graph with any insights"
    } else {
        "RUMINATION FOLLOW-UP — Continue improving the knowledge graph:\n\n\
         1. Review what you just changed — are there consequences or implications?\n\
         2. Do your changes create any new contradictions or competing hypotheses?\n\
         3. Are there related areas that could benefit from similar analysis?\n\
         4. Update the graph with any follow-on insights"
    };

    format!(
        "{}\n\n\
         IMPORTANT: Complete this follow-up task, update the graph files, \
         output a brief summary, and STOP. Do not ask follow-up questions.",
        followup
    )
}

/// Detect which AI backends are installed on this system.
/// Handle `/model` and `/backends` commands.
fn handle_model_command(message: &str, config: &CliWorkerConfig) -> String {
    let arg = message.strip_prefix("/model ")
        .or_else(|| message.strip_prefix("/backends "))
        .unwrap_or("")
        .trim();

    if let Some(ref registry) = config.backend_registry {
        if arg.is_empty() {
            let mut lines = vec!["**Available AI backends:**\n".to_string()];
            for b in registry.all() {
                let cats: Vec<String> = b.tags().categories.iter().map(|c| c.to_string()).collect();
                lines.push(format!(
                    "- **{}** — {} (quality:{}, speed:{}, cost:{})\n  Categories: {}",
                    b.id(), b.name(),
                    b.tags().quality_tier, b.tags().speed_tier, b.tags().cost_tier,
                    if cats.is_empty() { "none".into() } else { cats.join(", ") },
                ));
            }
            lines.push("\n**Categories:** cost_effective, intellectual, fast, local, balanced".into());
            lines.push("Usage: `/model <category>` to see which backends handle a category".into());
            lines.join("\n")
        } else {
            let resolved = registry.resolve(arg);
            if resolved.is_empty() {
                format!("No backends found for '{}'. Available: {}", arg,
                    registry.ids().join(", "))
            } else {
                let names: Vec<String> = resolved.iter()
                    .map(|b| format!("{}({})", b.id(), b.name()))
                    .collect();
                format!("**Backends for '{}':** {}\n\n(First available will be used, others are fallback)",
                    arg, names.join(" → "))
            }
        }
    } else {
        let installed = crate::backends::detect_backends();
        let lines: Vec<String> = installed.iter()
            .map(|(name, avail)| format!("- {} {}", name, if *avail { "✓" } else { "✗" }))
            .collect();
        format!("**AI backends (legacy mode):**\n{}\n\nCurrent: [{}]",
            lines.join("\n"),
            config.backends.join(", "))
    }
}

/// Run the AI worker loop.
///
/// Each incoming request is spawned as a concurrent task. Multiple
/// requests can be in flight simultaneously.
#[allow(clippy::too_many_arguments)]
pub async fn ai_worker_loop(
    mut rx: mpsc::UnboundedReceiver<CliRequest>,
    response_queue: Arc<Mutex<VecDeque<CliResponse>>>,
    config: CliWorkerConfig,
    stats: StatsMap,
    telegram_tx: mpsc::UnboundedSender<(i64, String)>,
    tasks: TaskMap,
    follow_ups: FollowUpQueue,
    request_tx: mpsc::UnboundedSender<CliRequest>,
    event_tx: Option<tokio::sync::broadcast::Sender<crate::registry::WsEvent>>,
    bot_name: String,
) {
    // Wrap config and bot_name in Arc to avoid cloning per-request.
    let config = Arc::new(config);
    let bot_name: Arc<str> = bot_name.into();

    // Per-source chat ID mapping for cross-channel forwarding.
    // Maps source ("telegram", "slack") → last known chat_id from that source.
    let mut source_chat_ids: HashMap<String, i64> = HashMap::new();

    // Ensure memory directory exists.
    if let Err(e) = std::fs::create_dir_all(&config.memory_dir) {
        log::warn!("Could not create memory dir {:?}: {}", config.memory_dir, e);
    }

    // Knowledge graph — accessed through the validated store.
    let knowledge_file = config.memory_dir.join("knowledge.json");
    let knowledge_store = crate::store::live::LiveKnowledgeStore::new(config.memory_dir.clone());
    // Keep CachedGraph for semantic rendering (uses Ollama embeddings).
    // TODO: Move semantic rendering into the store.
    let knowledge_cache = crate::knowledge::CachedGraph::new(&knowledge_file);

    // Ollama client for embeddings and as an AI backend.
    // Persistent embedding cache stored in the memory directory.
    let embed_cache_path = config.memory_dir.join("embeddings.json");
    let ollama_client = crate::ollama::OllamaClient::with_cache(None, None, embed_cache_path);

    // Backend session tracker for continuity.
    let backend_sessions: Arc<Mutex<BackendSessions>> = Arc::new(Mutex::new(BackendSessions::default()));

    // Episodic memory file.
    let episodes_file = config.memory_dir.join("episodes.json");

    // Periodic archiving of low-confidence edges (every 100 requests).
    let mut request_count: u32 = 0;
    let mut last_decay = Instant::now();

    // Track whether we've shown pending questions to this user in this session.
    let mut questions_shown_to: std::collections::HashSet<i64> = std::collections::HashSet::new();

    // Colony inbox polling interval.
    let mut colony_poll = tokio::time::interval(tokio::time::Duration::from_secs(5));
    colony_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Wait for either a request OR a colony inbox poll.
        let req = tokio::select! {
            Some(r) = rx.recv() => r,
            _ = colony_poll.tick() => {
                // Check colony inbox for messages from other ANTs.
                process_colony_inbox(&config.memory_dir, &request_tx, &bot_name);
                process_colony_outbox(&config.memory_dir, &bot_name);
                continue;
            }
        };

        let is_rumination = req.source == "rumination";
        let is_colony_query = req.source.starts_with("colony:");

        // Remember chat IDs per source for cross-channel forwarding.
        if req.chat_id != 0 && req.source != "web" && !is_rumination && !is_colony_query {
            source_chat_ids.insert(req.source.clone(), req.chat_id);
        }

        // Rumination and colony requests use dedicated memory files.
        let user_memory_file = if is_rumination {
            config.memory_dir.join("rumination.md")
        } else if is_colony_query {
            config.memory_dir.join("colony.md")
        } else {
            config.memory_dir.join(format!("{}.md", req.chat_id))
        };

        // Create user memory file if it doesn't exist.
        if !user_memory_file.exists() {
            let header = if is_rumination {
                "# Rumination Memory\n\nThis file is used by the rumination engine for autonomous thinking.\n\n".to_string()
            } else {
                format!("# Memory — user {}\n\n", req.chat_id)
            };
            let _ = std::fs::write(&user_memory_file, header);
        }

        // Present pending questions from rumination on first human interaction.
        if !is_rumination && req.chat_id > 0 && !questions_shown_to.contains(&req.chat_id) {
            questions_shown_to.insert(req.chat_id);
            let questions_file = config.memory_dir.join("questions.json");
            if let Some(questions_text) = load_and_clear_questions(&questions_file) {
                // Send as a bot message before processing the user's request.
                if let Some(ref tx) = event_tx {
                    let _ = tx.send(crate::registry::WsEvent::Message {
                        bot: bot_name.to_string(),
                        chat_id: req.chat_id,
                        text: questions_text.clone(),
                        task_id: 0,
                    });
                }
                // Also send to Telegram if applicable.
                let tg_chat = source_chat_ids.get("telegram").copied().unwrap_or(0);
                if tg_chat != 0 {
                    let _ = telegram_tx.send((tg_chat, questions_text.clone()));
                }
                // Push to response queue for R2 bus.
                if let Ok(mut q) = response_queue.lock() {
                    q.push_back(CliResponse {
                        chat_id: req.chat_id,
                        text: questions_text,
                        task_id: 0,
                    });
                }
            }
        }

        // Periodic maintenance via the store.
        request_count += 1;
        if request_count.is_multiple_of(50) {
            use crate::store::KnowledgeStore;
            // Consolidate all graphs: dedup, link orphans, backfill.
            if let Ok(graphs) = knowledge_store.list_graphs() {
                for g in &graphs {
                    let _ = knowledge_store.consolidate(&g.name);
                    let _ = knowledge_store.backfill_thurisaz(&g.name);
                    let _ = knowledge_store.link_orphans(&g.name);
                }
            }
            // Invalidate the CachedGraph so it picks up changes.
            knowledge_cache.invalidate();
        }
        if request_count.is_multiple_of(100) {
            // Archive low-confidence edges to separate file.
            knowledge_cache.archive_stale();
        }
        // Time-based confidence decay — runs on first request after 24h idle.
        let since_decay = last_decay.elapsed().as_secs();
        if since_decay > 86400 { // 24 hours
            use crate::store::KnowledgeStore;
            let days = (since_decay / 86400) as u32;
            if let Ok(graphs) = knowledge_store.list_graphs() {
                for g in &graphs {
                    let _ = knowledge_store.apply_decay(&g.name, days);
                }
            }
            knowledge_cache.invalidate();
            last_decay = Instant::now();
        }

        // --- /model and /backends — handled directly, no AI backend needed ---
        if req.message == "/model" || req.message == "/backends" || req.message.starts_with("/model ") {
            let reply = handle_model_command(&req.message, &config);
            if let Ok(mut q) = response_queue.lock() {
                q.push_back(CliResponse { chat_id: req.chat_id, text: reply.clone(), task_id: req.task_id });
            }
            if let Some(ref tx) = event_tx {
                let _ = tx.send(crate::registry::WsEvent::Message {
                    bot: bot_name.to_string(),
                    chat_id: req.chat_id,
                    text: reply.clone(),
                    task_id: req.task_id,
                });
            }
            let _ = telegram_tx.send((req.chat_id, reply));
            continue;
        }

        // --- Special commands ---
        let is_analytical = req.message.starts_with("/analyse ")
            || req.message == "/reflect"
            || req.message == "/compact-chat"
            || req.message.starts_with("/specify ")
            || req.message.starts_with("/test-vectors ");

        let actual_message = if let Some(path) = req.message.strip_prefix("/analyse ") {
            build_analyse_message(path.trim(), &config.working_dir, &knowledge_file)
        } else if req.message == "/reflect" {
            build_reflect_message(&knowledge_file)
        } else if req.message == "/compact-chat" {
            build_compact_chat_message(&config.memory_dir, &episodes_file, req.chat_id)
        } else if let Some(path) = req.message.strip_prefix("/specify ") {
            build_specify_message(path.trim(), &config.working_dir)
        } else if let Some(path) = req.message.strip_prefix("/test-vectors ") {
            build_test_vectors_message(path.trim(), &config.working_dir)
        } else {
            req.message.clone()
        };

        // For rumination requests: append a clear termination directive.
        // AI backends tend to ask "what next?" — rumination must be self-contained.
        let actual_message = if is_rumination {
            format!(
                "{}\n\n\
                 IMPORTANT: This is an autonomous rumination task. \
                 Complete the work described above, update the graph files, \
                 and then STOP. Do not ask follow-up questions. Do not ask \
                 what to do next. Do not wait for input. \
                 When you have finished updating the graph, output a brief \
                 summary of what you changed and stop.",
                actual_message
            )
        } else {
            actual_message
        };

        // Build the command for the selected backend.
        // Knowledge graph + episodes + user memory pre-loaded into the prompt.
        // Try semantic search (Ollama embeddings) first, fall back to keyword-based.
        let kg_rendered = if ollama_client.is_available().await {
            knowledge_cache.render_for_prompt_semantic(&ollama_client, &actual_message, 4096).await
        } else {
            knowledge_cache.render_for_prompt(&actual_message, 4096)
        };

        // Load relevant episodes.
        let episodes_mem = crate::knowledge::EpisodicMemory::load(&episodes_file);
        let relevant_episodes = episodes_mem.search(&actual_message, 5);
        let episodes_rendered = episodes_mem.render(&relevant_episodes, 2048);

        // Log message preview.
        log::info!("[{}] Message preview: {}", bot_name,
            if actual_message.len() > 80 {
                format!("{}...", &actual_message[..actual_message.floor_char_boundary(80)])
            } else {
                actual_message.clone()
            });

        let system_prompt = build_system_prompt(
            config.system_prompt.as_deref(),
            &knowledge_file,
            &kg_rendered,
            &episodes_rendered,
            &user_memory_file,
            &config.working_dir,
            &config.repos_dir,
            is_analytical,
            config.allow_base_code_changes,
        );
        let cfg = Arc::clone(&config);
        let message_for_backends = actual_message.clone();
        let system_prompt_for_backends = system_prompt;
        let continue_session = !req.new_session;
        let is_new_session = req.new_session;
        let input_len = req.message.len() as u64;
        let chat_id = req.chat_id;
        let task_id = req.task_id;
        let req_source = req.source.clone();
        let tg_chat = source_chat_ids.get("telegram").copied().unwrap_or(0);
        let rq = Arc::clone(&response_queue);
        let st = Arc::clone(&stats);
        let tm = Arc::clone(&tasks);
        let ttx = telegram_tx.clone();
        let etx = event_tx.clone();
        let bname = Arc::clone(&bot_name);
        let rq_tx = request_tx.clone();


        // Broadcast user message (for history and cross-device sync).
        // Skip for rumination — autonomous thinking is not user-facing.
        if req.source != "rumination" {
            if let Some(ref tx) = etx {
                let _ = tx.send(crate::registry::WsEvent::UserMessage {
                    bot: bname.to_string(),
                    chat_id,
                    text: req.message.clone(),
                    source: req.source.clone(),
                });
            }
        }

        // Forward user message to Telegram if from another channel and sync is enabled.
        if config.sync_channels && req.source != "telegram" && req.source != "rumination" && tg_chat != 0 {
            let label = match req.source.as_str() {
                "web" => "🌐 web",
                "slack" => "💬 slack",
                _ => &req.source,
            };
            let _ = ttx.send((tg_chat, format!("[{}] {}", label, req.message)));
        }

        // Broadcast task started event.
        let preview = if req.message.len() > 50 {
            truncate_safe(&req.message, 47)
        } else {
            req.message.clone()
        };
        if let Some(ref tx) = etx {
            let _ = tx.send(crate::registry::WsEvent::TaskStarted {
                bot: bname.to_string(),
                task_id,
                preview: preview.clone(),
            });
        }

        // Shared progress tracking — written by the spawned task, read by /status.
        let live_progress: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let live_backend: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let task_live_progress = Arc::clone(&live_progress);
        let task_live_backend = Arc::clone(&live_backend);
        let follow_ups_clone = Arc::clone(&follow_ups);
        let backend_sessions_clone = Arc::clone(&backend_sessions);

        // Spawn the task concurrently.
        let handle = tokio::spawn(async move {
            // Send typing indicator every 4 seconds.
            let typing_tx = ttx.clone();
            let typing_handle = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                    if typing_tx.send((chat_id, String::new())).is_err() {
                        break;
                    }
                }
            });

            // ── Registry-based backend execution ──────────────────────
            // When a BackendRegistry is available and [ai] config is set,
            // resolve backends through the registry (supports categories,
            // API backends, etc.).  Falls back to the upstream
            // strategy-based code path otherwise.
            let mut response_text = String::new();
            let mut _used_backend = String::new();

            let used_registry = if let Some(ref registry) = cfg.backend_registry {
                // Resolve backends: prefer [ai] config, fall back to legacy list,
                // fall back to all registered backends.
                let backends_to_try: Vec<_> = if let Some(ref ai_cfg) = cfg.ai_config {
                    // [ai] section exists — use category/explicit resolution.
                    let ids = ai_cfg.resolve_backends("");
                    log::info!("[{}] AI config active: default_category='{}', resolved ids={:?}",
                        bname, ai_cfg.default_category, ids);
                    if ids.is_empty() {
                        // [ai] exists but no category/backends set — map legacy names.
                        cfg.backends.iter()
                            .map(|b| crate::ai_backends::legacy_name_to_id(b))
                            .flat_map(|id| registry.resolve(&id))
                            .collect()
                    } else {
                        ids.into_iter()
                            .flat_map(|id| registry.resolve(&id))
                            .collect()
                    }
                } else if !cfg.backends.is_empty() {
                    // No [ai] config, but explicit [claude].backends list — map through registry.
                    cfg.backends.iter()
                        .map(|b| crate::ai_backends::legacy_name_to_id(b))
                        .flat_map(|id| registry.resolve(&id))
                        .collect()
                } else {
                    // No [ai] config AND no explicit backends list.
                    // Use all registered backends (auto-detected CLIs + ollama).
                    log::info!("[{}] No [ai] config and no explicit backends — using all {} registered backends",
                        bname, registry.all().len());
                    registry.all()
                };

                let backend_names: Vec<String> = backends_to_try.iter()
                    .map(|b| b.id().to_string()).collect();
                log::info!("[{}] Registry resolved {} backends: [{}]",
                    bname, backends_to_try.len(), backend_names.join(", "));

                if backends_to_try.is_empty() {
                    false
                } else {
                    let ai_request = crate::ai_backends::AiRequest {
                        task_id,
                        chat_id,
                        message: message_for_backends.clone(),
                        system_prompt: system_prompt_for_backends.clone(),
                        working_dir: cfg.working_dir.clone(),
                        skip_permissions: cfg.skip_permissions,
                        continue_session,
                        memory_dir: Some(std::path::PathBuf::from(&cfg.memory_dir)),
                        context: std::collections::HashMap::new(),
                    };

                    let mut all_errors: Vec<(String, String)> = Vec::new();

                    for (idx, backend) in backends_to_try.iter().enumerate() {
                        let backend_id = backend.id().to_string();
                        let backend_name = backend.name().to_string();
                        if let Ok(mut b) = task_live_backend.lock() { *b = backend_id.clone(); }
                        if let Ok(mut p) = task_live_progress.lock() {
                            *p = Some(format!("Starting {}...", backend_name));
                        }

                        let (progress_tx, mut progress_rx) =
                            tokio::sync::mpsc::unbounded_channel::<crate::ai_backends::AiProgress>();

                        let progress_etx = etx.clone();
                        let progress_bname = bname.clone();
                        let progress_task_live = Arc::clone(&task_live_progress);
                        let progress_tg_chat = tg_chat;
                        let progress_ttx = ttx.clone();
                        let progress_handle = tokio::spawn(async move {
                            while let Some(prog) = progress_rx.recv().await {
                                if let Ok(mut p) = progress_task_live.lock() {
                                    *p = Some(prog.detail.clone());
                                }
                                if prog.kind == "question" && progress_tg_chat != 0 {
                                    let _ = progress_ttx.send((progress_tg_chat,
                                        format!("[Task #{}] {}\n\nReply with /followup <answer>",
                                            prog.task_id, prog.detail)));
                                }
                                if let Some(ref tx) = progress_etx {
                                    let _ = tx.send(crate::registry::WsEvent::TaskProgress {
                                        bot: progress_bname.to_string(),
                                        task_id: prog.task_id,
                                        kind: prog.kind,
                                        detail: prog.detail,
                                    });
                                }
                            }
                        });

                        log::info!("[{}] Trying backend '{}' ({}/{})...",
                            bname, backend_id, idx + 1, backends_to_try.len());

                        let result = backend.execute(&ai_request, progress_tx).await;
                        progress_handle.abort();

                        match result {
                            Ok(resp) => {
                                log::info!("[{}] Backend '{}' succeeded", bname, backend_id);
                                response_text = resp.text;
                                _used_backend = resp.backend_id;
                                break;
                            }
                            Err(err) => {
                                log::warn!("[{}] Backend '{}' failed: {}", bname, backend_id, err);
                                all_errors.push((backend_id.clone(), err.message.clone()));

                                if err.retriable && idx + 1 < backends_to_try.len() {
                                    let next = backends_to_try[idx + 1].name();
                                    log::info!("[{}] Falling back to '{}'", bname, next);
                                    if let Some(ref tx) = etx {
                                        let _ = tx.send(crate::registry::WsEvent::TaskProgress {
                                            bot: bname.to_string(),
                                            task_id,
                                            kind: "fallback".into(),
                                            detail: format!("{} failed, trying {}...",
                                                backend_name, next),
                                        });
                                    }
                                } else {
                                    // All backends exhausted — show detailed error report.
                                    let error_report = all_errors.iter()
                                        .map(|(id, msg)| format!("• {}: {}", id, msg))
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    response_text = format!(
                                        "All {} backend(s) failed:\n\n{}\n\nTry /model to see which backends are available.",
                                        all_errors.len(), error_report
                                    );
                                    _used_backend = backend_id;
                                    log::error!("[{}] All backends failed:\n{}", bname, error_report);
                                }
                            }
                        }
                    }
                    true
                }
            } else {
                false
            };

            if response_text.is_empty() && !used_registry {
                response_text = "No AI backends available. Install Claude, Ollama, or configure API backends in [ai.backends_config].".to_string();
            }

            let _ = used_registry;


            typing_handle.abort();

            // Handle colony query responses — forward back to the originating ANT.
            if req_source.starts_with("colony:") {
                let parts: Vec<&str> = req_source.splitn(3, ':').collect();
                if parts.len() >= 3 {
                    let from_ant = parts[1];
                    let orig_chat_id: i64 = parts[2].parse().unwrap_or(0);

                    // 1. Show the response in the originating ANT's chat (for the human).
                    if let Some(ref tx) = etx {
                        let _ = tx.send(crate::registry::WsEvent::Message {
                            bot: from_ant.to_string(),
                            chat_id: orig_chat_id,
                            text: format!("**Response from {}:**\n\n{}", bname, response_text),
                            task_id: 0,
                        });
                    }

                    // 2. Send the evaluation task to the ORIGINATING ANT (not ourselves).
                    // Write to their colony inbox so their worker picks it up.
                    let ants_dir = cfg.memory_dir.parent()
                        .and_then(|p| p.parent())
                        .and_then(|p| p.parent());
                    if let Some(dir) = ants_dir {
                        let target_memory = dir.join(from_ant).join("working").join("memory");
                        // Also check ant.toml for custom working_dir.
                        let target_memory = resolve_ant_memory(dir, from_ant)
                            .unwrap_or(target_memory);
                        if target_memory.exists() {
                            let inbox = target_memory.join("colony_inbox");
                            let _ = std::fs::create_dir_all(&inbox);
                            let eval_msg = format!(
                                "COLONY RESPONSE from {} — engage in Socratic discourse:\n\n\
                                 {}\n\n\
                                 SOCRATIC METHOD — advance the conversation through:\n\
                                 1. EXAMINE: Does this response introduce NEW knowledge or insight?\n\
                                    If it repeats what you already know, note that and move on.\n\
                                 2. QUESTION: What assumptions does {} make? Are they justified?\n\
                                    Challenge weak claims with specific counter-evidence.\n\
                                 3. CONJECTURE: Formulate your own thesis in response — what do\n\
                                    YOU think, based on your expertise and this new input?\n\
                                 4. REFUTE: Try to disprove your own thesis. If it survives, it's\n\
                                    stronger. If it fails, say so honestly.\n\
                                 5. SYNTHESISE: If both perspectives have merit, propose a synthesis\n\
                                    that combines the strongest elements of each.\n\
                                 6. ADVANCE: End with a NEW question or direction — not a restatement.\n\
                                    If the topic is exhausted, say so.\n\n\
                                 Update your knowledge graph:\n\
                                 - Well-evidenced claims → add with source_id 'ant:{}', evidence_type 'corroboration'\n\
                                 - Claims that contradict your evidence → 'contradiction'\n\
                                 - Unsupported claims → 'inconsequential_search'\n\
                                 - Update {}'s expert_in edge based on response quality\n\n\
                                 CRITICAL — when to respond vs when to stop:\n\
                                 - If you have a substantive response that introduces NEW knowledge,\n\
                                   a NEW question, or a genuine disagreement backed by evidence,\n\
                                   send it back via colony_outbox.\n\
                                 - If you AGREE with the other ANT, or the topic is exhausted, or\n\
                                   you have nothing new to add: update your graph and STOP.\n\
                                   Do NOT write to colony_outbox. Do NOT send a message saying\n\
                                   you agree or that the discussion is complete — silence IS the\n\
                                   signal that the conversation has concluded.",
                                bname, response_text, bname, bname, bname
                            );
                            let inbox_msg = serde_json::json!({
                                "from": bname.to_string(),
                                "message": eval_msg,
                                "chat_id": orig_chat_id,
                                "timestamp": crate::dateutil::datetime_now(),
                            });
                            let filename = format!("response-{}-{}.json", bname,
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis());
                            let _ = std::fs::write(
                                inbox.join(&filename),
                                serde_json::to_string_pretty(&inbox_msg).unwrap_or_default()
                            );
                        }
                    }

                    log::info!("[{}] Colony query from {} complete ({} chars) — forwarded for evaluation",
                        bname, from_ant, response_text.len());
                }
            }

            // Update stats (skip for rumination and colony queries).
            if req_source != "rumination" && !req_source.starts_with("colony:") {
                if let Ok(mut map) = st.lock() {
                    let s = map.entry(chat_id).or_default();
                    if s.started.is_none() {
                        s.started = Some(Instant::now());
                    }
                    s.messages += 1;
                    s.input_chars += input_len;
                    s.output_chars += response_text.len() as u64;
                }
            }

            // Broadcast results to WebSocket (always — including rumination).
            // Rumination summaries appear in the chat so the human can see what
            // the ANT was thinking, whether they were watching or not.
            if let Some(ref tx) = etx {
                // For background rumination, use chat_id 0 so it appears as a
                // system message rather than being tied to a specific user.
                let broadcast_chat_id = if req_source == "rumination" && chat_id < 0 { 0 } else { chat_id };
                let _ = tx.send(crate::registry::WsEvent::Message {
                    bot: bname.to_string(),
                    chat_id: broadcast_chat_id,
                    text: response_text.clone(),
                    task_id,
                });
            }

            if req_source != "rumination" {
                // Forward response to Telegram if from another channel and sync is enabled.
                if cfg.sync_channels && req_source != "telegram" && tg_chat != 0 {
                    let _ = ttx.send((tg_chat, response_text.clone()));
                }

                // Push response to R2 event bus (for Telegram plugin).
                if let Ok(mut q) = rq.lock() {
                    q.push_back(CliResponse {
                        chat_id,
                        text: response_text.clone(),
                        task_id,
                    });
                }

                // Record backend session for continuity tracking (use actual backend, not old strategy).
                backend_sessions_clone.lock().unwrap().record_backend(chat_id, &_used_backend);
                
                // Store response summary for context injection when switching backends.
                if !response_text.is_empty() {
                    let summary = if response_text.len() > 500 {
                        let end = response_text.floor_char_boundary(500);
                        format!("{}... ({} chars)", &response_text[..end], response_text.len())
                    } else {
                        response_text.clone()
                    };
                    backend_sessions_clone.lock().unwrap().set_summary(chat_id, summary);
                }
            } else {
                log::info!("[{}] Rumination task #{} complete ({} chars)",
                    bname, task_id, response_text.len());
                // Broadcast graph update — the AI likely modified graph files.
                if let Some(ref tx) = etx {
                    let _ = tx.send(crate::registry::WsEvent::GraphUpdated {
                        bot: bname.to_string(),
                        graph: "all".into(),
                        source: "rumination".into(),
                    });
                }

                // Spawn a rumination follow-up based on what was just done.
                // Only spawn from top-level rumination (new_session=true), not from
                // follow-ups, to prevent infinite chains.
                if is_new_session && !response_text.is_empty() && response_text.len() > 50 {
                    let follow_up_prompt = build_rumination_followup(&response_text);
                    let _ = rq_tx.send(CliRequest {
                        chat_id,
                        message: follow_up_prompt,
                        new_session: false, // continue session for context
                        task_id: 0,
                        source: "rumination".into(),
                    });
                    log::info!("[{}] Spawned rumination follow-up from task #{}",
                        bname, task_id);
                }
            }

            // Remove from running tasks and broadcast completion.
            let duration_secs = {
                let mut dur = 0u64;
                if let Ok(mut map) = tm.lock() {
                    if let Some(task) = map.remove(&task_id) {
                        dur = task.started.elapsed().as_secs();
                        if let Ok(mut s) = task.state.lock() {
                            if *s == TaskState::Running {
                                *s = TaskState::Completed;
                            }
                        }
                    }
                }
                dur
            };
            if let Some(ref tx) = etx {
                let _ = tx.send(crate::registry::WsEvent::TaskCompleted {
                    bot: bname.to_string(),
                    task_id,
                    duration_secs,
                });
            }

            // Process follow-up queue — dispatch queued messages with session continuity.
            if let Ok(mut fq) = follow_ups_clone.lock() {
                if let Some(follow_ups) = fq.remove(&task_id) {
                    for fu in follow_ups {
                        log::info!("[{}] Dispatching follow-up for task #{}: {}",
                            bname, task_id,
                            if fu.message.len() > 50 { slice_safe(&fu.message, 47) } else { &fu.message });
                        // Re-queue as a new request (with session continuity via -c).
                        let _ = rq_tx.send(CliRequest {
                            chat_id: fu.chat_id,
                            message: fu.message,
                            new_session: false, // continue session
                            task_id: 0, // will be assigned by the worker loop
                            source: fu.source,
                        });
                    }
                }
            }
        });

        // Track the running task.
        if let Ok(mut map) = tasks.lock() {
            map.insert(
                task_id,
                RunningTask {
                    task_id,
                    chat_id,
                    message_preview: preview,
                    started: Instant::now(),
                    handle,
                    last_progress: live_progress,
                    backend: live_backend,
                    state: Arc::new(Mutex::new(TaskState::Running)),
                },
            );
        }
    }
}

/// Maximum system prompt size in bytes. Beyond this, dynamic context is truncated.
/// The preamble (~4KB) is always included; the remaining budget is split among
/// knowledge graph, episodes, and user memory by priority.
const MAX_SYSTEM_PROMPT: usize = 16_384;

#[allow(clippy::too_many_arguments)]
fn build_system_prompt(
    custom: Option<&str>,
    knowledge_file: &Path,
    kg_rendered: &str,
    episodes_rendered: &str,
    user_memory_file: &Path,
    working_dir: &str,
    repos_dir: &Path,
    is_analytical: bool,
    allow_base_code_changes: bool,
) -> String {
    let mut prompt = String::new();

    // File access restriction — before anything else.
    if !allow_base_code_changes {
        prompt.push_str(&format!(
            "RESTRICTION: You MUST NOT create, edit, or delete files outside your working directory ({}).\n\
            You may read files anywhere, but ONLY write within your workspace and repos/ subdirectory.\n\
            If asked to modify external code (e.g. Anthill source), explain what changes are needed\n\
            but do NOT make them. This restriction is enforced by policy.\n\n",
            working_dir
        ));
    }

    // Strict restriction on graph files — MCP tools only.
    prompt.push_str(
        "CRITICAL RESTRICTION — KNOWLEDGE GRAPH ACCESS:\n\
        You MUST NOT directly create, edit, or write to any file matching:\n\
          memory/knowledge.json, memory/knowledge.cbor\n\
          memory/graphs/*.json, memory/graphs/*.cbor\n\n\
        DO NOT use Python, jq, sed, or any script to read or modify graph files.\n\
        DO NOT parse graph JSON/CBOR with code. The graph files are managed by the\n\
        CBOR backend — direct edits are overwritten and lost.\n\n\
        ALL knowledge graph operations MUST go through the MCP tools:\n\
          graph_add_node       — add a concept\n\
          graph_add_edge       — add a relationship\n\
          graph_add_citation   — attach a source reference to an edge\n\
          graph_update_evidence — add typed evidence (corroboration, refutation, etc.)\n\
          graph_strengthen     — record survived refutation\n\
          graph_weaken         — record inconsistency\n\
          graph_contradict     — record failed refutation\n\
          graph_query_about    — explore around an entity\n\
          graph_query_uncertain — find low-confidence edges\n\
          graph_list_nodes     — list all nodes in a graph\n\n\
        These tools validate input, maintain Bayesian integrity, handle CBOR\n\
        serialisation, and auto-commit to git. There is no reason to bypass them.\n\n"
    );

    // Fixed sections always included.
    if let Some(custom) = custom {
        prompt.push_str(custom);
        prompt.push_str("\n\n");
    }

    prompt.push_str(WORKSPACE_PREAMBLE);
    prompt.push_str(&format!(
        "\n\nWorking directory: {}\nRepos directory: {}\n\n",
        working_dir,
        repos_dir.display()
    ));

    prompt.push_str(MEMORY_PREAMBLE);
    // Include methodology only for analytical commands — saves ~1KB per regular request.
    if is_analytical {
        prompt.push_str(METHODOLOGY_PREAMBLE);
    }
    prompt.push_str(&format!(
        "\nKnowledge graph: {}\nUser memory file: {}\n",
        knowledge_file.display(),
        user_memory_file.display()
    ));

    // Self-evolved thinking process — the ANT's own methodology refinements.
    // This file is a conjecture itself: the ANT can modify it to improve how it thinks.
    let thinking_process_file = knowledge_file.parent()
        .map(|p| p.join("thinking_process.md"))
        .unwrap_or_default();
    let thinking_process = std::fs::read_to_string(&thinking_process_file).unwrap_or_default();
    if !thinking_process.trim().is_empty() {
        prompt.push_str("\n[THINKING PROCESS — self-evolved methodology]\n");
        prompt.push_str(slice_safe(&thinking_process, 2048));
        prompt.push_str("\n[/THINKING PROCESS]\n");
    }

    // Dynamic sections — fit within remaining budget.
    // Knowledge graph gets the lion's share since it drives reasoning quality.
    let remaining = MAX_SYSTEM_PROMPT.saturating_sub(prompt.len());
    let kg_budget = remaining * 70 / 100;   // 70% — knowledge graph (primary context)
    let um_budget = remaining * 15 / 100;   // 15% — user memory (preferences)
    let ep_budget = remaining * 15 / 100;   // 15% — episodes (narrative color)

    // Knowledge graph context (highest priority).
    if !kg_rendered.is_empty() {
        prompt.push_str("\n[KNOWLEDGE GRAPH]\n");
        if kg_rendered.len() > kg_budget {
            prompt.push_str(slice_safe(kg_rendered, kg_budget));
            prompt.push_str("\n(Semantic search via embeddings.)\n");
        } else {
            prompt.push_str(kg_rendered);
        }
        prompt.push_str("[/KNOWLEDGE GRAPH]\n");
    }

    // Episodes.
    if !episodes_rendered.is_empty() {
        prompt.push_str("\n[EPISODES]\n");
        if episodes_rendered.len() > ep_budget {
            prompt.push_str(slice_safe(episodes_rendered, ep_budget));
            prompt.push_str("\n... (more episodes in episodes.json)\n");
        } else {
            prompt.push_str(episodes_rendered);
        }
        prompt.push_str("[/EPISODES]\n");
    }

    // User memory.
    let user_memory = std::fs::read_to_string(user_memory_file).unwrap_or_default();
    if !user_memory.trim().is_empty() {
        prompt.push_str("\n[USER MEMORY]\n");
        if user_memory.len() > um_budget {
            prompt.push_str(slice_safe(&user_memory, um_budget));
            prompt.push_str("\n... (truncated — read the full file for more)\n");
        } else {
            prompt.push_str(&user_memory);
        }
        prompt.push_str("\n[/USER MEMORY]\n");
    }

    if prompt.len() > MAX_SYSTEM_PROMPT {
        log::warn!(
            "System prompt exceeds budget: {} bytes (limit {})",
            prompt.len(), MAX_SYSTEM_PROMPT
        );
    }

    // Colony directory — pre-populated list of peer ANTs.
    // This saves the AI from having to call list_colony_ants every time.
    let colony_dir = build_colony_directory(working_dir);
    if !colony_dir.is_empty() {
        prompt.push_str("\n[COLONY — your peer ANTs]\n");
        prompt.push_str(&colony_dir);
        prompt.push_str("[/COLONY]\n");
    }

    prompt
}

/// Build a colony directory listing for the system prompt.
/// Scans the ants directory for peers and their topic graphs.
fn build_colony_directory(working_dir: &str) -> String {
    let working_path = std::path::Path::new(working_dir);
    let ants_dir = working_path.parent() // <ant>/working
        .and_then(|p| p.parent()); // <ant>

    let ants_parent = match ants_dir.and_then(|p| p.parent()) { // ants/
        Some(d) => d,
        None => return String::new(),
    };

    let self_name = ants_dir
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    let entries = match std::fs::read_dir(ants_parent) {
        Ok(e) => e,
        Err(_) => return String::new(),
    };

    let mut listing = String::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let name = path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        let is_self = name == self_name;
        let memory = path.join("working").join("memory");
        if !memory.exists() { continue; }

        // Quick scan of topic graphs.
        let mut topics = Vec::new();
        let graphs_dir = memory.join("graphs");
        if graphs_dir.exists() {
            if let Ok(files) = std::fs::read_dir(&graphs_dir) {
                for f in files.flatten() {
                    let fname = f.path();
                    let ext = fname.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext == "cbor" || ext == "json" {
                        let stem = fname.file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if !stem.contains("-archive") && !stem.contains(".corrupted") {
                            topics.push(stem);
                        }
                    }
                }
            }
        }

        if is_self {
            listing.push_str(&format!("- {} (you): {}\n", name,
                if topics.is_empty() { "no topic graphs".into() } else { topics.join(", ") }));
        } else {
            listing.push_str(&format!("- {}: {}\n", name,
                if topics.is_empty() { "no topic graphs yet".into() } else { topics.join(", ") }));
        }
    }

    if listing.is_empty() { return String::new(); }

    format!("You can consult these ANTs using query_ant or /ask:\n{}\n\
             Use query_ant for quick knowledge lookup. Use /ask (via the user) for \
             a real conversation where the other ANT thinks about your question.\n", listing)
}

/// Read a document, handling PDFs by extracting text via pdftotext.
fn read_document(path: &Path) -> Result<String, String> {
    let ext = path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "pdf" => {
            // Use pdftotext (poppler-utils) to extract text from PDF.
            let output = std::process::Command::new("pdftotext")
                .arg("-layout")
                .arg(path)
                .arg("-")  // stdout
                .output()
                .map_err(|e| format!("pdftotext not found (install poppler-utils): {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("pdftotext failed: {}", stderr));
            }
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if text.trim().is_empty() {
                return Err("PDF appears to be image-only (no extractable text). Try OCR first.".into());
            }
            Ok(text)
        }
        "docx" | "doc" => {
            // Try pandoc for Word documents.
            let output = std::process::Command::new("pandoc")
                .args(["-t", "plain"])
                .arg(path)
                .output()
                .map_err(|e| format!("pandoc not found (install pandoc for .docx support): {}", e))?;
            if !output.status.success() {
                return Err("pandoc conversion failed".into());
            }
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        _ => {
            // Plain text, markdown, source code, etc.
            std::fs::read_to_string(path)
                .map_err(|e| format!("Could not read file: {}", e))
        }
    }
}

/// Build the message for /analyse <file> — thematic analysis of a document.
fn build_analyse_message(file_path: &str, working_dir: &str, _knowledge_file: &Path) -> String {
    use crate::thematic;

    // Resolve the file path relative to working directory.
    let full_path = if file_path.starts_with('/') {
        std::path::PathBuf::from(file_path)
    } else {
        std::path::PathBuf::from(working_dir).join(file_path)
    };

    let content = match read_document(&full_path) {
        Ok(c) => c,
        Err(e) => return format!("Could not read '{}': {}", file_path, e),
    };

    let source_name = full_path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.to_string());

    let chunks = thematic::chunk_document(&content);

    if chunks.len() <= 2 {
        // Short document — single combined analysis prompt with explicit step-by-step.
        format!(
            r#"You MUST perform THEMATIC ANALYSIS (Braun & Clarke, 2022) on this document.
Document: "{source_name}"

Follow these phases IN ORDER. Complete each phase before starting the next.
Show your work for each phase.

PHASE 1 — FAMILIARISATION:
Read the entire document. Note what it's about, its structure, and key topics.
Write a 2-3 sentence overview.

PHASE 2 — CODING:
Extract every significant entity, concept, decision, tool, person, and fact.
For each code, state:
  - Label (short name)
  - Kind (person/project/server/tool/concept/decision/event/fact)
  - One-line summary
  - Evidence (quote or paraphrase from the document)

PHASE 3 — THEME GENERATION:
Group your codes into 3-8 themes. Each theme is a pattern of shared meaning.
For each theme, state:
  - Theme name
  - Central concept
  - Which codes belong to it
  - Support level (how well-evidenced: 0.0-1.0)

PHASE 4 — REVIEW:
Re-read the document. Check each theme against the source:
  - Does the evidence support the theme?
  - Did you miss any codes?
  - Are any themes too broad or too narrow?
Revise themes and codes as needed.

PHASE 5 — RELATIONSHIPS:
Identify relationships between entities. For each relationship:
  - From → To (must match code labels)
  - Relation type (uses, deployed_on, depends_on, part_of, decided, etc.)
  - View: semantic, temporal, causal, or entity
  - Basis: observed (explicit in text, confidence 0.7), inferred (implied, 0.4), or assumed (interpretation, 0.3)
  - Source: "{source_name}"

PHASE 6 — INTEGRATION:
Determine the topic name for this document (lowercase-hyphenated, e.g. "anthill-architecture").
Run: mkdir -p memory/graphs/
FIRST, read the existing memory/graphs/<topic>.json (if it exists).
Study its nodes and edges to understand:
  - What topics and entities are already captured
  - What themes already exist
  - What confidence levels existing edges have
Then integrate your findings:
  - If a code matches an existing node → update (don't duplicate). Strengthen edges
    that are confirmed (increment survived + tests). Update summary if richer.
  - If a code is new → add the node and its edges
  - If a theme fits an existing theme → add new codes as members
  - If a theme is new → add a concept node and link its codes with "part_of" edges
  - If a relationship already exists → strengthen (increment survived + tests)
  - If a relationship contradicts an existing edge → weaken the old one (tests only)
  - If a relationship is new → add with appropriate confidence
  - Add "{source_name}" as an event node with today's date
  - Set valid_from on all new edges to today's date
  - Write the updated memory/graphs/<topic>.json

Then update the META-GRAPH (memory/knowledge.json):
  - Add a node for this topic graph if not already there (kind: "concept", tags: ["graph", "topic"])
  - Add edges from this topic to related existing topics
  - Write the updated knowledge.json

Finally, summarise: what themes you found, how many nodes/edges added vs updated, which graph(s).

DOCUMENT:
{content}"#,
            source_name = source_name,
            content = content
        )
    } else {
        // Long document — full 6-phase thematic analysis.
        format!(
            r#"You MUST perform THEMATIC ANALYSIS (Braun & Clarke, 2022) on a large document.
Document: "{source_name}" ({n} chunks, located at: {path})

Follow these phases IN ORDER. Complete each phase before starting the next.
Show your work for each phase.

PHASE 1 — FAMILIARISATION:
Read the ENTIRE file at {path} — all of it, not just the first part.
Write a 2-3 sentence overview of what the document covers.

PHASE 2 — CODING:
Extract every significant entity, concept, decision, tool, person, and fact.
For each code, state:
  - Label (short name)
  - Kind (person/project/server/tool/concept/decision/event/fact)
  - One-line summary
  - Evidence (quote or paraphrase from the document)

PHASE 3 — THEME GENERATION:
Group your codes into 3-8 themes. Each theme is a pattern of shared meaning.
For each theme, state:
  - Theme name
  - Central concept
  - Which codes belong to it
  - Support level (how well-evidenced: 0.0-1.0)

PHASE 4 — REVIEW:
Re-read the document. Check each theme against the source:
  - Does the evidence support the theme?
  - Did you miss any codes?
  - Are any themes too broad or too narrow?
Revise themes and codes as needed.

PHASE 5 — RELATIONSHIPS:
Identify relationships between entities. For each relationship:
  - From → To (must match code labels)
  - Relation type (uses, deployed_on, depends_on, part_of, decided, etc.)
  - View: semantic, temporal, causal, or entity
  - Basis: observed (explicit in text, confidence 0.7), inferred (implied, 0.4), or assumed (interpretation, 0.3)
  - Source: "{source_name}"

PHASE 6 — INTEGRATION:
Determine the topic name for this document (lowercase-hyphenated).
Run: mkdir -p memory/graphs/
FIRST, read the existing memory/graphs/<topic>.json (if it exists).
Study its nodes and edges to understand:
  - What topics and entities are already captured
  - What themes already exist
  - What confidence levels existing edges have
Then integrate your findings:
  - If a code matches an existing node → update (don't duplicate). Strengthen edges
    that are confirmed (increment survived + tests). Update summary if richer.
  - If a code is new → add the node and its edges
  - If a theme fits an existing theme → add new codes as members
  - If a theme is new → add a concept node and link its codes with "part_of" edges
  - If a relationship already exists → strengthen (increment survived + tests)
  - If a relationship contradicts an existing edge → weaken the old one (tests only)
  - If a relationship is new → add with appropriate confidence
  - Add "{source_name}" as an event node with today's date
  - Set valid_from on all new edges to today's date
  - Write the updated memory/graphs/<topic>.json

Then update the META-GRAPH (memory/knowledge.json):
  - Add a node for this topic graph if not already there (kind: "concept", tags: ["graph", "topic"])
  - Add edges from this topic to related existing topics
  - Write the updated knowledge.json

Finally, summarise: what themes you found, how many nodes/edges added vs updated, which graph(s)."#,
            source_name = source_name,
            n = chunks.len(),
            path = full_path.display(),
        )
    }
}

/// Build the message for /reflect — meta-analysis of the knowledge graph.
fn build_reflect_message(knowledge_file: &Path) -> String {
    let kg_content = std::fs::read_to_string(knowledge_file).unwrap_or_default();
    let node_count = kg_content.matches("\"label\"").count();

    format!(
        r#"REFLECT on your knowledge graph (memory/knowledge.json).

The graph currently has approximately {node_count} nodes. Perform meta-analysis:

1. REVIEW ALL NODES AND EDGES — read memory/knowledge.json completely.

2. IDENTIFY PATTERNS:
   - Are there clusters of related nodes that should be linked by a theme/concept node?
   - Are there implicit relationships that should be made explicit?
   - Are there nodes that seem to be about the same thing but named differently? (merge them)

3. TEST CONJECTURES:
   - For each edge, does the current conversation history support or contradict it?
   - Strengthen edges that are well-supported (increment 'survived' and 'tests')
   - Weaken or mark edges that seem outdated (increment 'tests' only)

4. DETECT CONTRADICTIONS:
   - Are there edges between the same nodes that conflict?
   - Flag these by adding a "contradiction" event node linked to both

5. ASSESS IMPORTANCE:
   - Which relationships are central to the project? (increase importance)
   - Which are peripheral? (decrease importance)

6. CONSOLIDATE:
   - Merge duplicate nodes (keep the better summary, union tags)
   - Collapse trivial chains (A→B→C where B adds nothing)
   - Remove orphan nodes with no edges

7. Write the updated knowledge.json and briefly summarise what changed.

Current graph location: {path}"#,
        node_count = node_count,
        path = knowledge_file.display()
    )
}

/// Build the message for /specify <file> — generate a specification from code.
fn build_specify_message(file_path: &str, working_dir: &str) -> String {
    let full_path = if file_path.starts_with('/') {
        std::path::PathBuf::from(file_path)
    } else {
        std::path::PathBuf::from(working_dir).join(file_path)
    };

    let content = match read_document(&full_path) {
        Ok(c) => c,
        Err(e) => return format!("Could not read '{}': {}", file_path, e),
    };

    let file_name = full_path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.to_string());

    let spec_name = file_name.replace('.', "-").to_uppercase();

    format!(
        r#"Generate a FORMAL SPECIFICATION from this source code.

Follow the Anthill specification style (RFC 2119, numbered sections, terminology table).

Process:
1. Read the code and identify all behaviors, invariants, and contracts
2. Group them into logical specification sections
3. Write each behavior as a normative statement (MUST/SHOULD/MAY)
4. Include a security considerations section
5. Save the specification as specs/{spec_name}.md

Source file: {file_name}

CODE:
{content}"#,
        spec_name = spec_name,
        file_name = file_name,
        content = if content.len() > 30000 { slice_safe(&content, 30000) } else { &content }
    )
}

/// Build the message for /test-vectors <file> — generate test cases.
fn build_test_vectors_message(file_path: &str, working_dir: &str) -> String {
    let full_path = if file_path.starts_with('/') {
        std::path::PathBuf::from(file_path)
    } else {
        std::path::PathBuf::from(working_dir).join(file_path)
    };

    let content = match read_document(&full_path) {
        Ok(c) => c,
        Err(e) => return format!("Could not read '{}': {}", file_path, e),
    };

    let file_name = full_path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.to_string());

    let is_spec = file_name.ends_with(".md") && content.contains("MUST");
    let is_code = file_name.ends_with(".rs") || file_name.ends_with(".py")
        || file_name.ends_with(".ts") || file_name.ends_with(".go");

    let source_type = if is_spec { "specification" } else if is_code { "source code" } else { "document" };

    format!(
        r#"Generate TEST VECTORS from this {source_type}.

For each behavior/requirement found, generate:
- A normal/happy path test
- An edge case or boundary test
- An error/negative test (if applicable)
- A security test (if relevant)

Output format: for each test, provide:
- Test name (snake_case, suitable for #[test])
- Description
- Setup / preconditions
- Input
- Expected output/behavior
- Category: normal, edge, error, security

If this is source code, also generate runnable Rust #[test] stubs.
If this is a specification, generate tests that verify the spec requirements.

Source file: {file_name}
Type: {source_type}

CONTENT:
{content}"#,
        source_type = source_type,
        file_name = file_name,
        content = if content.len() > 30000 { slice_safe(&content, 30000) } else { &content }
    )
}

/// Build the message for /compact-chat — analyse conversation, extract to graph, trim history.
fn build_compact_chat_message(memory_dir: &Path, episodes_file: &Path, chat_id: i64) -> String {
    let user_mem_path = memory_dir.join(format!("{}.md", chat_id));
    let user_memory = std::fs::read_to_string(&user_mem_path).unwrap_or_default();
    let episodes = std::fs::read_to_string(episodes_file).unwrap_or_default();

    format!(
        r#"You MUST perform THEMATIC ANALYSIS (Braun & Clarke, 2022) on this conversation.
Compact it into the knowledge graph. Follow these phases IN ORDER. Show your work.

PHASE 1 — FAMILIARISATION:
Review the entire conversation history (your session context).
Write a 2-3 sentence overview of what was discussed.

PHASE 2 — CODING:
Extract every significant entity, concept, decision, tool, person, and fact.
For each code, state:
  - Label (short name)
  - Kind (person/project/server/tool/concept/decision/event/fact)
  - One-line summary
  - Evidence (quote or paraphrase from the conversation)

PHASE 3 — THEME GENERATION:
Group your codes into 3-8 themes. Each theme is a pattern of shared meaning.
For each theme: name, central concept, which codes belong, support level (0.0-1.0).

PHASE 4 — REVIEW:
Check each theme against the conversation. Did you miss anything?
Are any themes too broad or too narrow? Revise as needed.

PHASE 5 — RELATIONSHIPS:
Identify relationships between entities. For each:
  - From → To (must match code labels)
  - Relation type (uses, deployed_on, depends_on, decided, etc.)
  - View: semantic, temporal, causal, or entity
  - Basis: told (user stated it, confidence 0.6), observed (you saw evidence, 0.7),
    inferred (implied, 0.4), or assumed (interpretation, 0.3)
  - Source: "conversation {chat_id}"

PHASE 6 — INTEGRATION:
Run: mkdir -p memory/graphs/
FIRST, read the existing memory/graphs/conversation.json (if it exists).
Study its nodes and edges to understand:
  - What topics and entities are already captured
  - What themes already exist
  - What confidence levels existing edges have
Then for each code from Phase 2:
  - If it matches an existing node → update (don't duplicate). Strengthen edges
    that are confirmed (increment survived + tests). Update summary if richer.
  - If it's new → add the node and its edges
For each theme from Phase 3:
  - If it fits an existing theme → add the new codes as members
  - If it's a new theme → add a concept node and link its codes
For each relationship from Phase 5:
  - If it already exists → strengthen (increment survived + tests)
  - If it contradicts an existing edge → weaken the old one (increment tests only)
  - If it's new → add with appropriate confidence
Set valid_from to today, source to "conversation-compact".
Write the updated memory/graphs/conversation.json.
Then update the meta-graph (knowledge.json):
  - Ensure a node exists for "conversation" (kind: "concept", tags: ["graph", "topic"])
  - Add edges between "conversation" and any related topic graphs
  - Write knowledge.json

PHASE 7 — EPISODE:
Write an episode to {episodes_path}:
  - date: today
  - participants: ["user:{chat_id}"]
  - summary: 2-3 sentences about what was discussed and decided
  - outcomes: key decisions or results
  - tags: searchable keywords
  - entities: labels of entities mentioned (linking to graph nodes)

PHASE 8 — TRIM MEMORY:
Update {user_mem_path}:
  - Keep only the last 3-4 key points as context for next time
  - Add: "Earlier conversation compacted to knowledge graph on [today's date]"
  - Remove entries now captured in the graph

Finally, summarise: what themes you found, how many nodes/edges added, which graph(s).

CURRENT USER MEMORY:
{user_memory}

RECENT EPISODES:
{episodes}"#,
        chat_id = chat_id,
        episodes_path = episodes_file.display(),
        user_mem_path = user_mem_path.display(),
        user_memory = slice_safe(&user_memory, 4000),
        episodes = slice_safe(&episodes, 2000),
    )
}

// ── Colony inbox/outbox processing ──────────────────────────────────

/// Resolve an ANT's memory directory, checking ant.toml for custom working_dir.
fn resolve_ant_memory(ants_dir: &std::path::Path, ant_name: &str) -> Option<std::path::PathBuf> {
    let config_path = ants_dir.join(ant_name).join("ant.toml");
    if let Ok(contents) = std::fs::read_to_string(&config_path) {
        if let Ok(cfg) = toml::from_str::<crate::config::Config>(&contents) {
            if let Some(wd) = &cfg.claude.working_dir {
                return Some(std::path::PathBuf::from(wd).join(&cfg.claude.memory_dir));
            }
        }
    }
    None
}

/// Process the colony inbox — pick up messages from other ANTs.
/// Called on a 5-second poll interval, not just on requests.
/// Track recent colony exchanges to detect loops.
/// Persisted as a simple JSON file so it survives restarts.
fn check_colony_loop(memory_dir: &std::path::Path, from: &str, message: &str) -> bool {
    let tracker_path = memory_dir.join("colony_tracker.json");
    let mut tracker: Vec<(String, String)> = if tracker_path.exists() {
        std::fs::read_to_string(&tracker_path).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // 1. Hard cap: too many exchanges with this ANT → stop.
    let exchange_count = tracker.iter().filter(|(f, _)| f == from).count();
    if exchange_count >= MAX_COLONY_EXCHANGES {
        log::info!("Colony exchange cap reached ({} messages from {})", exchange_count, from);
        // Clear tracker for this ANT so future conversations can start fresh.
        tracker.retain(|(f, _)| f != from);
        let _ = std::fs::write(&tracker_path, serde_json::to_string(&tracker).unwrap_or_default());
        return true;
    }

    // 2. Conclusion detection: if the message signals the discussion is over,
    //    don't deliver it — the other ANT would just agree back.
    let lower = message.to_lowercase();
    let conclusion_phrases = [
        "discussion is complete", "conversation is complete", "exchange is complete",
        "nothing new to add", "nothing further to add", "no new insights",
        "agree with your assessment", "agree with your conclusion",
        "we are in agreement", "we're in agreement",
        "topic is exhausted", "topic has been exhausted",
        "covered all the key", "covered the key points",
        "no further points", "no additional insights",
        "this concludes", "concludes our discussion",
        "thank you for the exchange", "thank you for this exchange",
        "productive exchange", "productive discussion",
    ];
    let is_conclusion = conclusion_phrases.iter().any(|p| lower.contains(p));

    // 3. Word overlap: if the last 2 messages from the same ANT share >60%
    //    of significant words with the current message, it's a loop.
    let recent_from_same: Vec<&str> = tracker.iter().rev()
        .filter(|(f, _)| f == from)
        .take(3)
        .map(|(_, m)| m.as_str())
        .collect();

    let is_word_loop = if recent_from_same.len() >= 2 {
        let current_words: std::collections::HashSet<&str> = message
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .collect();
        if current_words.is_empty() { false }
        else {
            recent_from_same.iter().all(|prev| {
                let prev_words: std::collections::HashSet<&str> = prev
                    .split_whitespace()
                    .filter(|w| w.len() > 3)
                    .collect();
                if prev_words.is_empty() { return false; }
                let overlap = current_words.intersection(&prev_words).count();
                let total = current_words.len().max(prev_words.len());
                (overlap as f64 / total as f64) > 0.6
            })
        }
    } else {
        false
    };

    let is_loop = is_conclusion || is_word_loop;

    // Record this message.
    tracker.push((from.to_string(), message.chars().take(200).collect()));
    // Keep last 20 entries.
    if tracker.len() > 20 { tracker.drain(..tracker.len() - 20); }
    let _ = std::fs::write(&tracker_path, serde_json::to_string(&tracker).unwrap_or_default());

    is_loop
}

/// Maximum colony exchanges per ANT pair before forcing a conclusion.
const MAX_COLONY_EXCHANGES: usize = 6;

fn process_colony_inbox(
    memory_dir: &std::path::Path,
    request_tx: &tokio::sync::mpsc::UnboundedSender<CliRequest>,
    bot_name: &str,
) {
    let inbox_dir = memory_dir.join("colony_inbox");
    if !inbox_dir.exists() { return; }

    let entries = match std::fs::read_dir(&inbox_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().map(|e| e == "json").unwrap_or(false) { continue; }
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&contents) {
                let from = msg.get("from").and_then(|f| f.as_str()).unwrap_or("unknown");
                let message = msg.get("message").and_then(|m| m.as_str()).unwrap_or("");
                let orig_chat_id = msg.get("chat_id").and_then(|c| c.as_i64()).unwrap_or(0);

                if !message.is_empty() {
                    // Loop detection — stop repetitive exchanges.
                    if check_colony_loop(memory_dir, from, message) {
                        log::warn!("[{}] Colony loop detected with {} — stopping exchange", bot_name, from);
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }

                    let _ = request_tx.send(CliRequest {
                        chat_id: -2,
                        message: message.to_string(),
                        new_session: true,
                        task_id: 0,
                        source: format!("colony:{}:{}", from, orig_chat_id),
                    });
                    log::info!("[{}] Colony inbox: message from {}", bot_name, from);
                }
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// Process the colony outbox — forward messages to target ANTs' inboxes.
fn process_colony_outbox(
    memory_dir: &std::path::Path,
    bot_name: &str,
) {
    let outbox_dir = memory_dir.join("colony_outbox");
    if !outbox_dir.exists() { return; }

    let entries = match std::fs::read_dir(&outbox_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let filename = path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        // Parse the message — two formats supported:
        // 1. to-<ANT>.md — simple plain text (preferred, easy for AI)
        // 2. <name>.json — JSON with from/to/message fields (legacy)
        let (to, from, message, chat_id) = if filename.starts_with("to-") &&
            (filename.ends_with(".md") || filename.ends_with(".txt"))
        {
            // Simple format: filename = "to-Gaea.md", content = plain text message
            let target = filename
                .strip_prefix("to-").unwrap_or("")
                .strip_suffix(".md").or_else(|| filename.strip_prefix("to-")?.strip_suffix(".txt"))
                .unwrap_or("")
                .to_string();
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            (target, bot_name.to_string(), content, 0i64)
        } else if filename.ends_with(".json") {
            // JSON format
            let contents = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => { let _ = std::fs::remove_file(&path); continue; }
            };
            match serde_json::from_str::<serde_json::Value>(&contents) {
                Ok(msg) => {
                    let to = msg.get("to").and_then(|t| t.as_str()).unwrap_or("").to_string();
                    let from = msg.get("from").and_then(|f| f.as_str()).unwrap_or(bot_name).to_string();
                    let message = msg.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
                    let chat_id = msg.get("chat_id").and_then(|c| c.as_i64()).unwrap_or(0);
                    (to, from, message, chat_id)
                }
                Err(_) => { let _ = std::fs::remove_file(&path); continue; }
            }
        } else {
            // Unknown format — skip.
            continue;
        };

        if !to.is_empty() && !message.is_empty() {
            let ants_dir = memory_dir.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent());
            if let Some(dir) = ants_dir {
                // Check ant.toml for custom working_dir.
                let target_memory = resolve_ant_memory(dir, &to)
                    .unwrap_or_else(|| dir.join(&to).join("working").join("memory"));
                if target_memory.exists() {
                    let colony_msg = format!(
                        "COLONY MESSAGE from {}\n\n{}\n\n\
                         RULES OF DISCOURSE (Socratic dialectic with Popperian refutation):\n\
                         Each exchange must ADVANCE the discussion:\n\
                         1. If you agree, add NEW information or a new angle\n\
                         2. If you disagree, state a clear counter-thesis with evidence\n\
                         3. If you see a synthesis, propose it and move to the next question\n\
                         4. If the topic is exhausted, say so\n\
                         5. If you have nothing new to add, say so and STOP\n\n\
                         Your response will be forwarded back to {}.\n\
                         IMPORTANT: Advance the conversation, then STOP.",
                        from, message, from
                    );
                    let target_inbox = target_memory.join("colony_inbox");
                    let _ = std::fs::create_dir_all(&target_inbox);
                    let inbox_msg = serde_json::json!({
                        "from": from,
                        "message": colony_msg,
                        "chat_id": chat_id,
                        "timestamp": crate::dateutil::datetime_now(),
                    });
                    let inbox_file = format!("{}-{}.json", from,
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis());
                    let _ = std::fs::write(
                        target_inbox.join(&inbox_file),
                        serde_json::to_string_pretty(&inbox_msg).unwrap_or_default()
                    );
                    log::info!("[{}] Colony outbox: forwarded to {} inbox", bot_name, to);
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }
}
