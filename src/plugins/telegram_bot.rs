//! TelegramPlugin — bridges Telegram Bot API to R2 events.
//!
//! Incoming Telegram messages → RELAY_INPUT events.
//! Commands:
//!   0x01 — send text message (data = CBOR { 0: text, 1: uint(chat_id) })
//!   0x02 — send monospace message (same payload, wrapped in ```...```)

use r2_engine::plugin::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use teloxide::prelude::*;
use teloxide::types::{ChatAction, ChatId, ParseMode};
use tokio::sync::mpsc;

use crate::events::{RELAY_COMMAND, RELAY_INPUT};

/// Commands accepted by TelegramPlugin.
pub const CMD_SEND_TEXT: u8 = 0x01;
pub const CMD_SEND_MONO: u8 = 0x02;

/// An incoming message from Telegram.
struct IncomingMessage {
    text: String,
    chat_id: i64,
}

/// Shared message queue — data plane between Telegram and Claude plugins.
/// Full message text stored here; events carry only IDs.
pub type MessageQueue = Arc<Mutex<VecDeque<(i64, String)>>>;

pub struct TelegramPlugin {
    id: PluginId,
    incoming_rx: mpsc::Receiver<IncomingMessage>,
    outgoing_tx: mpsc::UnboundedSender<(i64, String)>,
    /// Pre-encoded CBOR buffer for poll().
    poll_buf: Vec<u8>,
    /// When true, emit RELAY_COMMAND instead of RELAY_INPUT.
    ai_mode: bool,
    /// Data plane: full message text stored here for the Claude plugin to consume.
    message_queue: MessageQueue,
}

impl TelegramPlugin {
    /// Create the plugin and spawn the bot in the background.
    ///
    /// `rt` is the tokio runtime handle for spawning the bot task.
    /// `allowed_chat_ids` restricts which chats can interact.
    pub fn new(id: PluginId, rt: &tokio::runtime::Handle, allowed_chat_ids: Vec<i64>, ai_mode: bool, message_queue: MessageQueue) -> Self {
        let (in_tx, in_rx) = mpsc::channel::<IncomingMessage>(64);
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<(i64, String)>();

        let allowed = allowed_chat_ids.clone();

        rt.spawn(async move {
            let bot = Bot::from_env();

            // Spawn outgoing message sender.
            let bot_out = bot.clone();
            tokio::spawn(async move {
                while let Some((chat_id, text)) = out_rx.recv().await {
                    // Empty text = send typing indicator.
                    if text.is_empty() {
                        let _ = bot_out
                            .send_chat_action(ChatId(chat_id), ChatAction::Typing)
                            .await;
                        continue;
                    }

                    let html = markdown_to_telegram_html(&text);
                    let result = bot_out
                        .send_message(ChatId(chat_id), &html)
                        .parse_mode(ParseMode::Html)
                        .await;
                    // If HTML parse fails (malformed), fall back to plain text.
                    if result.is_err() {
                        let _ = bot_out.send_message(ChatId(chat_id), &text).await;
                    }
                }
            });

            // Incoming message handler.
            let handler = Update::filter_message().endpoint(
                move |msg: Message, _bot: Bot| {
                    let tx = in_tx.clone();
                    let allowed = allowed.clone();
                    async move {
                        if let Some(text) = msg.text() {
                            let chat_id = msg.chat.id.0;
                            if allowed.is_empty() || allowed.contains(&chat_id) {
                                let _ = tx
                                    .send(IncomingMessage {
                                        text: text.to_string(),
                                        chat_id,
                                    })
                                    .await;
                            }
                        }
                        respond(())
                    }
                },
            );

            Dispatcher::builder(bot, handler)
                .build()
                .dispatch()
                .await;
        });

        Self {
            id,
            incoming_rx: in_rx,
            outgoing_tx: out_tx,
            poll_buf: Vec::new(),
            ai_mode,
            message_queue,
        }
    }

    /// Get a clone of the outgoing message sender.
    ///
    /// Used by the chunker sentant to send large payloads directly,
    /// bypassing the engine's 256-byte PayloadBuf limit.
    pub fn outgoing_sender(&self) -> mpsc::UnboundedSender<(i64, String)> {
        self.outgoing_tx.clone()
    }

    fn send_message(&self, chat_id: i64, text: &str) {
        let _ = self.outgoing_tx.send((chat_id, text.to_string()));
    }
}

impl Plugin for TelegramPlugin {
    fn execute(&mut self, command: PluginCommand, data: &[u8]) -> PluginResult {
        // Decode CBOR payload: { 0: text, 1: uint(chat_id) }
        let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
        let chat_id;
        let text;

        match (|| -> Option<(String, i64)> {
            let r2_cbor::Item::Map(2) = dec.next().ok()? else { return None };
            // key 0
            let r2_cbor::Item::UInt(0) = dec.next().ok()? else { return None };
            let r2_cbor::Item::Text(t) = dec.next().ok()? else { return None };
            let t = std::str::from_utf8(t).ok()?.to_string();
            // key 1
            let r2_cbor::Item::UInt(1) = dec.next().ok()? else { return None };
            let r2_cbor::Item::UInt(cid) = dec.next().ok()? else { return None };
            Some((t, cid as i64))
        })() {
            Some((t, cid)) => {
                text = t;
                chat_id = cid;
            }
            None => {
                return PluginResult::Error(PluginError::new(1, "invalid CBOR payload"));
            }
        }

        match command {
            CMD_SEND_TEXT => {
                self.send_message(chat_id, &text);
                PluginResult::Ok(PluginResponse::empty())
            }
            CMD_SEND_MONO => {
                let mono = format!("```\n{}\n```", text);
                self.send_message(chat_id, &mono);
                PluginResult::Ok(PluginResponse::empty())
            }
            _ => PluginResult::Error(PluginError::new(0xFF, "unknown command")),
        }
    }

    fn name(&self) -> &str {
        "telegram"
    }

    fn id(&self) -> PluginId {
        self.id
    }

    fn poll(&mut self) -> Option<(u32, &[u8])> {
        match self.incoming_rx.try_recv() {
            Ok(msg) => {
                if self.ai_mode {
                    // AI/Claude mode: store full text in data plane, emit small event.
                    // Parse command type from the text.
                    let cmd_type = classify_command(&msg.text);
                    let cancel_task_id = parse_cancel_id(&msg.text);

                    // Store full text in the shared message queue (data plane).
                    if let Ok(mut q) = self.message_queue.lock() {
                        q.push_back((msg.chat_id, msg.text));
                    }

                    // Emit small event: { 0: uint(cmd_type), 1: uint(chat_id), 2: uint(cancel_task_id) }
                    self.poll_buf.clear();
                    self.poll_buf.push(0xA3); // map(3)
                    self.poll_buf.push(0x00); // key 0
                    self.poll_buf.push(cmd_type); // cmd_type fits in single byte
                    self.poll_buf.push(0x01); // key 1
                    encode_uint(&mut self.poll_buf, msg.chat_id as u64);
                    self.poll_buf.push(0x02); // key 2
                    encode_uint(&mut self.poll_buf, cancel_task_id as u64);

                    Some((RELAY_COMMAND, &self.poll_buf))
                } else {
                    // Raw mode: carry text in the event (for terminal sentant).
                    self.poll_buf.clear();
                    let text_bytes = msg.text.as_bytes();
                    self.poll_buf.push(0xA2); // map(2)
                    self.poll_buf.push(0x00); // key 0
                    let len = text_bytes.len();
                    if len <= 23 {
                        self.poll_buf.push(0x60 | len as u8);
                    } else if len <= 255 {
                        self.poll_buf.push(0x78);
                        self.poll_buf.push(len as u8);
                    } else {
                        self.poll_buf.push(0x79);
                        self.poll_buf.extend_from_slice(&(len as u16).to_be_bytes());
                    }
                    self.poll_buf.extend_from_slice(text_bytes);
                    self.poll_buf.push(0x01); // key 1
                    encode_uint(&mut self.poll_buf, msg.chat_id as u64);

                    Some((RELAY_INPUT, &self.poll_buf))
                }
            }
            Err(_) => None,
        }
    }
}

/// Convert markdown to Telegram-compatible HTML.
///
/// Two-pass approach:
///   1. Extract fenced code blocks (``` ```) verbatim
///   2. Process remaining lines: headings, bullets, inline formatting
///
/// Supports: headings, bold, italic, inline code, code blocks,
/// bullet lists, numbered lists, links, strikethrough, horizontal rules.
/// Classify a user message into a command type for the event payload.
fn classify_command(text: &str) -> u8 {
    let trimmed = text.trim();
    match trimmed {
        "/help" | "/start" => 1,
        "/ants" | "/bots" | "/status" => 2,
        "/usage" => 3,
        "/new" => 6,
        s if s == "/cancel all" => 5,
        s if s == "/cancel" || s.starts_with("/cancel ") => 4,
        _ => 0, // regular message
    }
}

/// Parse a cancel task ID from "/cancel 42".
fn parse_cancel_id(text: &str) -> u32 {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("/cancel ") {
        let rest = rest.trim();
        if rest != "all" {
            return rest.parse().unwrap_or(0);
        }
    }
    0
}

fn encode_uint(buf: &mut Vec<u8>, v: u64) {
    if v <= 23 {
        buf.push(v as u8);
    } else if v <= 0xFF {
        buf.push(0x18);
        buf.push(v as u8);
    } else if v <= 0xFFFF {
        buf.push(0x19);
        buf.extend_from_slice(&(v as u16).to_be_bytes());
    } else if v <= 0xFFFF_FFFF {
        buf.push(0x1A);
        buf.extend_from_slice(&(v as u32).to_be_bytes());
    } else {
        buf.push(0x1B);
        buf.extend_from_slice(&v.to_be_bytes());
    }
}

fn markdown_to_telegram_html(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + 256);
    let mut lines = md.lines().peekable();

    while let Some(line) = lines.next() {
        // Fenced code block.
        if line.trim_start().starts_with("```") {
            out.push_str("<pre><code>");
            // Collect lines until closing ```.
            let mut code = String::new();
            loop {
                match lines.next() {
                    Some(l) if l.trim_start().starts_with("```") => break,
                    Some(l) => {
                        if !code.is_empty() {
                            code.push('\n');
                        }
                        code.push_str(l);
                    }
                    None => break,
                }
            }
            out.push_str(&html_escape(&code));
            out.push_str("</code></pre>\n");
            continue;
        }

        // Horizontal rule.
        let trimmed = line.trim();
        if (trimmed.starts_with("---") || trimmed.starts_with("***") || trimmed.starts_with("___"))
            && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_' || c == ' ')
            && trimmed.len() >= 3
        {
            out.push_str("——————————\n");
            continue;
        }

        // Headings: different levels get different styling.
        //   #    → bold with spacing
        //   ##   → bold
        //   ###  → bold italic
        //   #### → italic
        if trimmed.starts_with('#') {
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            let text = trimmed[hashes..].trim();
            let escaped = format_inline(&html_escape(text));
            match hashes {
                1 => {
                    out.push('\n');
                    out.push_str(&format!("<b>{}</b>\n", escaped.to_uppercase()));
                }
                2 => {
                    out.push('\n');
                    out.push_str(&format!("<b>{}</b>\n", escaped));
                }
                3 => {
                    out.push_str(&format!("<b><i>{}</i></b>\n", escaped));
                }
                _ => {
                    out.push_str(&format!("<i>{}</i>\n", escaped));
                }
            }
            continue;
        }

        // Bullet lists: - or * at start of line (with optional indent).
        if let Some(rest) = strip_bullet(trimmed) {
            out.push_str("• ");
            out.push_str(&format_inline(&html_escape(rest)));
            out.push('\n');
            continue;
        }

        // Regular line — apply inline formatting.
        out.push_str(&format_inline(&html_escape(line)));
        out.push('\n');
    }

    // Trim trailing newline.
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Strip a bullet prefix (- or * followed by space). Returns the rest of the line.
fn strip_bullet(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("- ") {
        Some(rest)
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        Some(rest)
    } else {
        None
    }
}

/// Apply inline formatting to an already HTML-escaped string.
///
/// Order matters: code first (protect contents), then bold, italic,
/// strikethrough, links.
fn format_inline(s: &str) -> String {
    let mut result = s.to_string();

    // Inline code: `...` → <code>...</code>
    result = replace_delimited(&result, "`", "`", "code");

    // Bold: **...** → <b>...</b>
    result = replace_delimited(&result, "**", "**", "b");

    // Italic: *...* → <i>...</i> (but not ** which is already handled)
    result = replace_single_asterisk(&result);

    // Strikethrough: ~~...~~ → <s>...</s>
    result = replace_delimited(&result, "~~", "~~", "s");

    // Links: [text](url) → <a href="url">text</a>
    result = replace_links(&result);

    result
}

/// Replace paired delimiters with HTML tags.
fn replace_delimited(s: &str, open: &str, close: &str, tag: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;

    loop {
        if let Some(start) = rest.find(open) {
            let after_open = start + open.len();
            if let Some(end) = rest[after_open..].find(close) {
                let content = &rest[after_open..after_open + end];
                result.push_str(&rest[..start]);
                result.push('<');
                result.push_str(tag);
                result.push('>');
                result.push_str(content);
                result.push_str("</");
                result.push_str(tag);
                result.push('>');
                rest = &rest[after_open + end + close.len()..];
                continue;
            }
        }
        result.push_str(rest);
        break;
    }
    result
}

/// Handle single * italic (after ** bold has been processed).
fn replace_single_asterisk(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;

    loop {
        if let Some(start) = rest.find('*') {
            // Skip if inside an HTML tag.
            if rest[..start].ends_with('<') || rest[start + 1..].starts_with('/') {
                result.push_str(&rest[..=start]);
                rest = &rest[start + 1..];
                continue;
            }
            if let Some(end) = rest[start + 1..].find('*') {
                let content = &rest[start + 1..start + 1 + end];
                // Only wrap if content doesn't span too far (sanity check).
                if !content.contains('\n') && content.len() < 200 {
                    result.push_str(&rest[..start]);
                    result.push_str("<i>");
                    result.push_str(content);
                    result.push_str("</i>");
                    rest = &rest[start + 1 + end + 1..];
                    continue;
                }
            }
        }
        result.push_str(rest);
        break;
    }
    result
}

/// Convert markdown links [text](url) → <a href="url">text</a>.
fn replace_links(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;

    loop {
        if let Some(bracket_start) = rest.find('[') {
            if let Some(bracket_end) = rest[bracket_start..].find("](") {
                let abs_bracket_end = bracket_start + bracket_end;
                if let Some(paren_end) = rest[abs_bracket_end + 2..].find(')') {
                    let text = &rest[bracket_start + 1..abs_bracket_end];
                    let url = &rest[abs_bracket_end + 2..abs_bracket_end + 2 + paren_end];
                    result.push_str(&rest[..bracket_start]);
                    result.push_str("<a href=\"");
                    result.push_str(url);
                    result.push_str("\">");
                    result.push_str(text);
                    result.push_str("</a>");
                    rest = &rest[abs_bracket_end + 2 + paren_end + 1..];
                    continue;
                }
            }
        }
        result.push_str(rest);
        break;
    }
    result
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
