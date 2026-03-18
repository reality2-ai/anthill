//! PtyPlugin — spawns and manages a pseudo-terminal.
//!
//! Bridges async PTY I/O to the synchronous Plugin trait via mpsc channels.
//!
//! Commands:
//!   0x01 — spawn PTY (data = shell path as UTF-8)
//!   0x02 — write to PTY stdin (data = raw bytes)
//!   0x03 — kill PTY

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use r2_engine::plugin::*;
use tokio::sync::mpsc;

use crate::events::RELAY_PTY_RAW;

/// Commands accepted by PtyPlugin.
pub const CMD_SPAWN: u8 = 0x01;
pub const CMD_WRITE: u8 = 0x02;
pub const CMD_KILL: u8 = 0x03;

/// PTY output chunk size (fits in PayloadBuf).
const OUTPUT_CHUNK: usize = 200;

pub struct PtyPlugin {
    id: PluginId,
    shell: String,
    master: Option<Arc<Mutex<Box<dyn MasterPty + Send>>>>,
    writer: Option<Box<dyn Write + Send>>,
    output_rx: mpsc::Receiver<Vec<u8>>,
    output_tx: mpsc::Sender<Vec<u8>>,
    /// Pre-encoded CBOR output buffer for poll().
    poll_buf: Vec<u8>,
    alive: Arc<Mutex<bool>>,
}

impl PtyPlugin {
    pub fn new(id: PluginId, shell: &str) -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self {
            id,
            shell: shell.to_string(),
            master: None,
            writer: None,
            output_rx: rx,
            output_tx: tx,
            poll_buf: Vec::new(),
            alive: Arc::new(Mutex::new(false)),
        }
    }

    fn spawn_pty(&mut self) -> PluginResult {
        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(e) => {
                return PluginResult::Error(PluginError::new(1, &format!("openpty: {e}")));
            }
        };

        let mut cmd = CommandBuilder::new(&self.shell);
        cmd.env("TERM", "dumb");

        if let Err(e) = pair.slave.spawn_command(cmd) {
            return PluginResult::Error(PluginError::new(2, &format!("spawn: {e}")));
        }

        let writer = match pair.master.take_writer() {
            Ok(w) => w,
            Err(e) => {
                return PluginResult::Error(PluginError::new(3, &format!("clone writer: {e}")));
            }
        };

        let mut reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(e) => {
                return PluginResult::Error(PluginError::new(4, &format!("clone reader: {e}")));
            }
        };

        self.master = Some(Arc::new(Mutex::new(pair.master)));
        self.writer = Some(writer);
        *self.alive.lock().unwrap() = true;

        // Spawn background reader thread (PTY reads are blocking).
        let tx = self.output_tx.clone();
        let alive = self.alive.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; OUTPUT_CHUNK];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            *alive.lock().unwrap() = false;
        });

        PluginResult::Ok(PluginResponse::empty())
    }

    fn write_pty(&mut self, data: &[u8]) -> PluginResult {
        if let Some(ref mut writer) = self.writer {
            match writer.write_all(data) {
                Ok(()) => PluginResult::Ok(PluginResponse::empty()),
                Err(e) => PluginResult::Error(PluginError::new(5, &format!("write: {e}"))),
            }
        } else {
            PluginResult::Error(PluginError::new(6, "no active PTY"))
        }
    }

    fn kill_pty(&mut self) -> PluginResult {
        self.writer = None;
        self.master = None;
        *self.alive.lock().unwrap() = false;
        PluginResult::Ok(PluginResponse::empty())
    }
}

impl Plugin for PtyPlugin {
    fn execute(&mut self, command: PluginCommand, data: &[u8]) -> PluginResult {
        match command {
            CMD_SPAWN => self.spawn_pty(),
            CMD_WRITE => self.write_pty(data),
            CMD_KILL => self.kill_pty(),
            _ => PluginResult::Error(PluginError::new(0xFF, "unknown command")),
        }
    }

    fn name(&self) -> &str {
        "pty"
    }

    fn id(&self) -> PluginId {
        self.id
    }

    fn poll(&mut self) -> Option<(u32, &[u8])> {
        match self.output_rx.try_recv() {
            Ok(data) => {
                // Encode as CBOR: { 0: bytes(data) }
                // Hand-encoded since Encoder works on fixed &mut [u8].
                self.poll_buf.clear();
                self.poll_buf.push(0xA1); // map(1)
                self.poll_buf.push(0x00); // key: 0
                // bstr header
                let len = data.len();
                if len <= 23 {
                    self.poll_buf.push(0x40 | len as u8);
                } else if len <= 255 {
                    self.poll_buf.push(0x58);
                    self.poll_buf.push(len as u8);
                } else {
                    self.poll_buf.push(0x59);
                    self.poll_buf.extend_from_slice(&(len as u16).to_be_bytes());
                }
                self.poll_buf.extend_from_slice(&data);
                Some((RELAY_PTY_RAW, &self.poll_buf))
            }
            Err(_) => None,
        }
    }
}
