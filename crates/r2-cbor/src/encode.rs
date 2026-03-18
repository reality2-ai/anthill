//! CBOR Compact mode encoder. Zero-allocation, single-pass.
//!
//! See SPEC.md §3–§5 for encoding rules.

use crate::Error;

/// A CBOR value for encoding (SPEC.md §3–§5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    /// Unsigned integer (major type 0).
    UInt(u64),
    /// Negative integer (major type 1). Value must be < 0.
    NegInt(i64),
    /// Boolean (simple values true/false).
    Bool(bool),
    /// Null (simple value 22).
    Null,
    /// IEEE 754 float32 (SPEC.md §5).
    Float32(f32),
    /// IEEE 754 float64 (SPEC.md §5).
    Float64(f64),
    /// Raw float16 bits — caller is responsible for IEEE 754 encoding.
    Float16Raw(u16),
    /// UTF-8 text string (major type 3).
    Text(&'a str),
    /// Byte string (major type 2).
    Bytes(&'a [u8]),
}

/// Fixed-buffer CBOR encoder (SPEC.md §3–§4).
///
/// Writes CBOR into a caller-provided `&mut [u8]` buffer. No heap allocation.
/// Returns [`Error::BufferFull`] if the buffer is exhausted.
pub struct Encoder<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Encoder<'a> {
    /// Create an encoder over a mutable buffer.
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Write a map header (n key-value pairs).
    #[inline]
    pub fn map(&mut self, n: usize) -> Result<(), Error> {
        self.head(5, n as u64)
    }

    /// Write an array header (n items).
    #[inline]
    pub fn array(&mut self, n: usize) -> Result<(), Error> {
        self.head(4, n as u64)
    }

    /// Write a uint key followed by a value (Compact mode convenience).
    #[inline]
    pub fn kv(&mut self, key: u64, val: &Value) -> Result<(), Error> {
        self.uint(key)?;
        self.value(val)
    }

    /// Write a single value.
    pub fn value(&mut self, val: &Value) -> Result<(), Error> {
        match *val {
            Value::UInt(v) => self.uint(v),
            Value::NegInt(v) => self.negint(v),
            Value::Bool(true) => self.byte(0xf5),
            Value::Bool(false) => self.byte(0xf4),
            Value::Null => self.byte(0xf6),
            Value::Float32(v) => {
                self.byte(0xfa)?;
                self.raw(&v.to_bits().to_be_bytes())
            }
            Value::Float64(v) => {
                self.byte(0xfb)?;
                self.raw(&v.to_bits().to_be_bytes())
            }
            Value::Float16Raw(bits) => {
                self.byte(0xf9)?;
                self.raw(&bits.to_be_bytes())
            }
            Value::Text(s) => {
                let b = s.as_bytes();
                self.head(3, b.len() as u64)?;
                self.raw(b)
            }
            Value::Bytes(b) => {
                self.head(2, b.len() as u64)?;
                self.raw(b)
            }
        }
    }

    /// Write an unsigned integer.
    #[inline]
    pub fn uint(&mut self, val: u64) -> Result<(), Error> {
        self.head(0, val)
    }

    /// Write a negative integer (val must be < 0).
    #[inline]
    pub fn negint(&mut self, val: i64) -> Result<(), Error> {
        self.head(1, (-1 - val) as u64)
    }

    /// Write a text string.
    pub fn text(&mut self, s: &str) -> Result<(), Error> {
        let b = s.as_bytes();
        self.head(3, b.len() as u64)?;
        self.raw(b)
    }

    /// Return the encoded bytes so far.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.pos]
    }

    /// Bytes written.
    #[inline]
    pub fn len(&self) -> usize {
        self.pos
    }

    /// Returns `true` if no bytes have been written.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pos == 0
    }

    // ── internals ───────────────────────────────────────────────────

    fn head(&mut self, major: u8, val: u64) -> Result<(), Error> {
        let mt = major << 5;
        if val <= 23 {
            self.byte(mt | val as u8)
        } else if val <= 0xFF {
            self.byte(mt | 24)?;
            self.byte(val as u8)
        } else if val <= 0xFFFF {
            self.byte(mt | 25)?;
            self.raw(&(val as u16).to_be_bytes())
        } else if val <= 0xFFFF_FFFF {
            self.byte(mt | 26)?;
            self.raw(&(val as u32).to_be_bytes())
        } else {
            self.byte(mt | 27)?;
            self.raw(&val.to_be_bytes())
        }
    }

    #[inline]
    fn byte(&mut self, b: u8) -> Result<(), Error> {
        if self.pos >= self.buf.len() {
            return Err(Error::BufferFull);
        }
        self.buf[self.pos] = b;
        self.pos += 1;
        Ok(())
    }

    #[inline]
    fn raw(&mut self, data: &[u8]) -> Result<(), Error> {
        if self.pos + data.len() > self.buf.len() {
            return Err(Error::BufferFull);
        }
        self.buf[self.pos..self.pos + data.len()].copy_from_slice(data);
        self.pos += data.len();
        Ok(())
    }
}
