//! UTF-8 agnostic parser implementation for SSE

use core::str::Utf8Error;

use bytes::{Buf, Bytes, BytesMut};
use bytes_utils::Str;

use crate::constants::{CR, LF};

/// A full line from an SSE stream
#[derive(Debug, Clone, Copy)]
pub enum RawEventLine<'a> {
    Comment, // we choose to ignore this since the OG code never uses it anyways
    Field {
        field_name: &'a [u8],
        field_value: Option<&'a [u8]>,
    },
    Empty,
}

/// Full line from an SSE stream, owned version of [RawEventLine]. Note: You probably want to [RawEventLineOwned::validate] these into [ValidatedEventLine]s
#[derive(Debug, Clone)]
pub enum RawEventLineOwned {
    Comment,
    Empty,
    Field {
        field_name: Bytes,
        field_value: Option<Bytes>,
    },
}

/// Valid field names according to [html.spec.whatwg.org](https://html.spec.whatwg.org/multipage/server-sent-events.html#event-stream-interpretation), invalid field names are thrown away into [FieldName::Ignored]
#[derive(Debug, Clone, Copy)]
pub enum FieldName {
    Event,
    Data,
    Id,
    Retry,
    Ignored,
}

/// Completely parsed SSE event line
#[derive(Debug, Clone)]
pub enum ValidatedEventLine {
    Comment,
    Empty,
    Field {
        field_name: FieldName,
        field_value: Option<Str>,
    },
}

fn validate_bytes(val: Bytes) -> Result<Str, Utf8Error> {
    match str::from_utf8(val.as_ref()) {
        Ok(_) => Ok(unsafe { Str::from_inner_unchecked(val) }),
        Err(e) => Err(e),
    }
}

impl RawEventLineOwned {
    pub fn validate(self) -> Result<ValidatedEventLine, core::str::Utf8Error> {
        match self {
            RawEventLineOwned::Comment => Ok(ValidatedEventLine::Comment),
            RawEventLineOwned::Empty => Ok(ValidatedEventLine::Empty),
            RawEventLineOwned::Field {
                field_name,
                field_value,
            } => {
                let field_name = match field_name.as_ref() {
                    b"event" => FieldName::Event,
                    b"data" => FieldName::Data,
                    b"id" => FieldName::Id,
                    b"retry" => FieldName::Retry,
                    _ => FieldName::Ignored,
                };

                let field_value = match field_value {
                    Some(b) => Some(validate_bytes(b)?),
                    None => None,
                };

                Ok(ValidatedEventLine::Field {
                    field_name,
                    field_value,
                })
            }
        }
    }
}

/// Splits a slice at the next EOL bytes, returns a tuple where the first value is the non-inclusive end of the line and the second value is the inclusive start of the remainder.
/// Returns [None] if more data is required to find the next EOL / an EOL byte is not found.
fn find_eol(bytes: &[u8]) -> Option<(usize, usize)> {
    let first_match = memchr::memchr2(CR, LF, bytes)?;

    match bytes[first_match] {
        LF => Some((first_match, first_match + 1)),
        CR => {
            if first_match + 1 >= bytes.len() {
                return None; // need more data to see if it's CRLF or just CR
            }

            // Cr lf
            if bytes[first_match + 1] == LF {
                Some((first_match, first_match + 2))
            } else {
                // just cr
                Some((first_match, first_match + 1))
            }
        }
        _ => unreachable!(),
    }
}

/// Splits a slice of bytes at the next EOL bytes. Returns None if more data is required to find the next EOL / an EOL byte is not found.
fn split_at_next_eol(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    find_eol(bytes).map(|(line_end, rem_start)| (&bytes[..line_end], &bytes[rem_start..]))
}

fn read_line(bytes: &[u8]) -> RawEventLine<'_> {
    match memchr::memchr(b':', bytes) {
        Some(colon_pos) => {
            if colon_pos == 0 {
                RawEventLine::Comment
            } else {
                let value = &bytes[colon_pos + 1..];
                // strip single leading space if present

                // ngl i found this syntax out from claude, pattern matching is crazy
                let value = match value {
                    [b' ', rest @ ..] => rest,
                    _ => value,
                };
                RawEventLine::Field {
                    field_name: &bytes[..colon_pos],
                    field_value: Some(value),
                }
            }
        }
        None => {
            if bytes.is_empty() {
                RawEventLine::Empty
            } else {
                RawEventLine::Field {
                    field_name: bytes,
                    field_value: None,
                }
            }
        }
    }
}

/// Tries to read the next [RawEventLine] from `bytes`. Returns [None] if `bytes` contains no complete EOL, this includes a slice ending in just cr as we are not yet sure if it's crlf or just a lone cr.
pub fn parse_line(bytes: &[u8]) -> Option<(RawEventLine<'_>, &[u8])> {
    let (line_to_read, next) = split_at_next_eol(bytes)?;
    Some((read_line(line_to_read), next))
}

/// Reads the next [RawEventLineOwned] from the buffer, then advances the buffer past the corresponding EOL.
/// Returns [None] if the buffer contains no cr, lf or crlf. Additionally returns [None] if the buffer ends with a cr as it could end up being a crlf if more data is added.
pub fn parse_line_from_buffer(buffer: &mut BytesMut) -> Option<RawEventLineOwned> {
    let (line_end, rem_start) = find_eol(buffer)?;

    let line = buffer.split_to(line_end).freeze();
    buffer.advance(rem_start - line_end);

    if line.is_empty() {
        return Some(RawEventLineOwned::Empty);
    }

    match memchr::memchr(b':', &line) {
        Some(0) => Some(RawEventLineOwned::Comment),
        Some(colon_pos) => {
            let value_start = if line.get(colon_pos + 1) == Some(&b' ') {
                colon_pos + 2
            } else {
                colon_pos + 1
            };
            Some(RawEventLineOwned::Field {
                field_name: line.slice(..colon_pos),
                field_value: Some(line.slice(value_start..)),
            })
        }
        None => Some(RawEventLineOwned::Field {
            field_name: line,
            field_value: None,
        }),
    }
}

pub fn parse_line_from_bytes(buffer: &mut Bytes) -> Option<RawEventLineOwned> {
    let (line_end, rem_start) = find_eol(buffer)?;

    let line = buffer.split_to(line_end);
    buffer.advance(rem_start - line_end);

    if line.is_empty() {
        return Some(RawEventLineOwned::Empty);
    }

    match memchr::memchr(b':', &line) {
        Some(0) => Some(RawEventLineOwned::Comment),
        Some(colon_pos) => {
            let value_start = if line.get(colon_pos + 1) == Some(&b' ') {
                colon_pos + 2
            } else {
                colon_pos + 1
            };
            Some(RawEventLineOwned::Field {
                field_name: line.slice(..colon_pos),
                field_value: Some(line.slice(value_start..)),
            })
        }
        None => Some(RawEventLineOwned::Field {
            field_name: line,
            field_value: None,
        }),
    }
}
