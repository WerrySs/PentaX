// SPDX-License-Identifier: GPL-2.0-only
// Original bounded, fallible JSON structural index. Strings remain in input;
// only explicitly retained source fields are decoded into owned byte vectors.

use super::*;

const ABSENT: usize = usize::MAX;
const MAX_NODES: usize = 65_536;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Null,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Real,
    String,
    Array,
    Object,
}
#[derive(Clone, Copy)]
struct Node {
    kind: Kind,
    start: usize,
    end: usize,
    child: usize,
    next: usize,
}

pub(super) struct Document<'a> {
    bytes: &'a [u8],
    nodes: Vec<Node>,
    pub root: usize,
}

enum Error {
    Invalid,
    Resource(Failure),
}
impl From<Failure> for Error {
    fn from(value: Failure) -> Self {
        Self::Resource(value)
    }
}
type ResultJson<T> = Result<T, Error>;

struct Parser<'a, 'b, 'c> {
    bytes: &'a [u8],
    at: usize,
    nodes: Vec<Node>,
    ctx: &'b mut Context<'c>,
}

impl<'a> Document<'a> {
    pub fn parse(bytes: &'a [u8], ctx: &mut Context<'_>) -> Result<Option<Self>, Failure> {
        let mut parser = Parser {
            bytes,
            at: usize::from(bytes.starts_with(&[0xef, 0xbb, 0xbf])) * 3,
            nodes: Vec::new(),
            ctx,
        };
        let parsed = parser.value(0).and_then(|root| {
            parser.space()?;
            if parser.at == bytes.len() {
                Ok(root)
            } else {
                Err(Error::Invalid)
            }
        });
        match parsed {
            Ok(root) => Ok(Some(Self {
                bytes,
                nodes: parser.nodes,
                root,
            })),
            Err(Error::Invalid) => {
                parser
                    .ctx
                    .release(parser.nodes.capacity() * core::mem::size_of::<Node>());
                Ok(None)
            }
            Err(Error::Resource(error)) => Err(error),
        }
    }
    pub fn metadata_bytes(&self) -> usize {
        self.nodes.capacity() * core::mem::size_of::<Node>()
    }
    pub fn kind(&self, node: usize) -> Kind {
        self.nodes[node].kind
    }
    pub fn array(&self, node: usize) -> Option<Children<'_, 'a>> {
        matches!(self.kind(node), Kind::Array).then(|| Children {
            doc: self,
            at: self.nodes[node].child,
        })
    }
    pub fn object(&self, node: usize) -> Option<Fields<'_, 'a>> {
        matches!(self.kind(node), Kind::Object).then(|| Fields {
            doc: self,
            at: self.nodes[node].child,
        })
    }
    pub fn field(
        &self,
        object: usize,
        key: &[u8],
        ctx: &mut Context<'_>,
    ) -> Result<Option<usize>, Failure> {
        let Some(fields) = self.object(object) else {
            return Ok(None);
        };
        let mut found = None;
        for (name, value) in fields {
            if self.eq(name, key, ctx)? {
                found = Some(value);
            }
        }
        Ok(found) // JSON objects retain the LAST duplicate member.
    }
    pub fn eq(&self, node: usize, value: &[u8], ctx: &mut Context<'_>) -> Result<bool, Failure> {
        if !matches!(self.kind(node), Kind::String) {
            return Ok(false);
        }
        ctx.work((self.nodes[node].end - self.nodes[node].start) as u64 + value.len() as u64 + 1)?;
        Ok(self.string(node).eq(value.iter().copied()))
    }
    pub fn same_key(
        &self,
        left: usize,
        right: usize,
        ctx: &mut Context<'_>,
    ) -> Result<bool, Failure> {
        ctx.work(
            (self.nodes[left].end - self.nodes[left].start + self.nodes[right].end
                - self.nodes[right].start) as u64
                + 1,
        )?;
        Ok(self.string(left).eq(self.string(right)))
    }
    pub fn last_key(
        &self,
        key: usize,
        value: usize,
        ctx: &mut Context<'_>,
    ) -> Result<bool, Failure> {
        let mut next = self.nodes[value].next;
        while next != ABSENT {
            if self.same_key(key, next, ctx)? {
                return Ok(false);
            }
            next = self.nodes[self.nodes[next].next].next;
        }
        Ok(true)
    }
    pub fn sole_field(
        &self,
        object: usize,
        key: &[u8],
        ctx: &mut Context<'_>,
    ) -> Result<Option<usize>, Failure> {
        let Some(fields) = self.object(object) else {
            return Ok(None);
        };
        let mut value = None;
        for (name, at) in fields {
            if !self.eq(name, key, ctx)? {
                return Ok(None);
            }
            value = Some(at);
        }
        Ok(value)
    }
    pub fn copy_string(
        &self,
        node: usize,
        ctx: &mut Context<'_>,
    ) -> Result<Option<Vec<u8>>, Failure> {
        if !matches!(self.kind(node), Kind::String) {
            return Ok(None);
        }
        let maximum = self.nodes[node].end - self.nodes[node].start;
        if maximum > 1024 * 1024 {
            return Err(Failure::Limit("Trophy JSON string bytes"));
        }
        let mut bytes = Vec::new();
        ctx.reserve(&mut bytes, maximum)?;
        ctx.work(maximum as u64 + 1)?;
        bytes.extend(self.string(node));
        Ok(Some(bytes))
    }
    pub fn unsigned(&self, node: usize, ctx: &mut Context<'_>) -> Result<Option<u64>, Failure> {
        match self.kind(node) {
            Kind::Unsigned(value) => Ok(Some(value)),
            Kind::Signed(value) if value >= 0 => Ok(Some(value as u64)),
            Kind::String => {
                ctx.work((self.nodes[node].end - self.nodes[node].start) as u64 + 1)?;
                Ok(decimal(self.string(node), false))
            }
            _ => Ok(None),
        }
    }
    pub fn id(&self, node: usize, ctx: &mut Context<'_>) -> Result<i32, Failure> {
        let value = if matches!(self.kind(node), Kind::String) {
            ctx.work((self.nodes[node].end - self.nodes[node].start) as u64 + 1)?;
            decimal(self.string(node), true)
        } else {
            self.unsigned(node, ctx)?
        };
        Ok(value
            .filter(|value| *value <= i32::MAX as u64)
            .map_or(-1, |value| value as i32))
    }
    fn string(&self, node: usize) -> StringBytes<'_> {
        let node = self.nodes[node];
        StringBytes {
            bytes: &self.bytes[node.start..node.end],
            at: 0,
            pending: [0; 4],
            index: 0,
            count: 0,
        }
    }
}

pub(super) fn decimal(bytes: impl Iterator<Item = u8>, signed_id: bool) -> Option<u64> {
    let mut value = 0u64;
    let mut any = false;
    let mut negative = false;
    let mut first = true;
    for byte in bytes {
        if first && signed_id && byte == b'-' {
            negative = true;
            first = false;
            continue;
        }
        first = false;
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
        any = true;
    }
    (any && (!negative || value == 0)).then_some(value)
}

pub(super) struct Children<'d, 'a> {
    doc: &'d Document<'a>,
    at: usize,
}
impl Iterator for Children<'_, '_> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        if self.at == ABSENT {
            return None;
        }
        let at = self.at;
        self.at = self.doc.nodes[at].next;
        Some(at)
    }
}
pub(super) struct Fields<'d, 'a> {
    doc: &'d Document<'a>,
    at: usize,
}
impl Iterator for Fields<'_, '_> {
    type Item = (usize, usize);
    fn next(&mut self) -> Option<Self::Item> {
        if self.at == ABSENT {
            return None;
        }
        let key = self.at;
        let value = self.doc.nodes[key].next;
        self.at = self.doc.nodes[value].next;
        Some((key, value))
    }
}

impl Parser<'_, '_, '_> {
    fn take(&mut self) -> ResultJson<u8> {
        self.ctx.work(1)?;
        let byte = *self.bytes.get(self.at).ok_or(Error::Invalid)?;
        self.at += 1;
        Ok(byte)
    }
    fn space(&mut self) -> ResultJson<()> {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|byte| matches!(byte, b' ' | b'\r' | b'\n' | b'\t'))
        {
            self.take()?;
        }
        Ok(())
    }
    fn node(&mut self, kind: Kind, start: usize, end: usize) -> ResultJson<usize> {
        if self.nodes.len() == MAX_NODES {
            return Err(Failure::Limit("Trophy JSON nodes").into());
        }
        let needed = self.nodes.len() + 1;
        self.ctx.reserve(&mut self.nodes, needed)?;
        let at = self.nodes.len();
        self.nodes.push(Node {
            kind,
            start,
            end,
            child: ABSENT,
            next: ABSENT,
        });
        Ok(at)
    }
    fn text(&mut self) -> ResultJson<usize> {
        if self.take()? != b'"' {
            return Err(Error::Invalid);
        }
        let start = self.at;
        loop {
            match self.take()? {
                b'"' => {
                    let end = self.at - 1;
                    self.ctx.work((end - start) as u64 + 1)?;
                    if std::str::from_utf8(&self.bytes[start..end]).is_err() {
                        return Err(Error::Invalid);
                    }
                    return self.node(Kind::String, start, end);
                }
                b'\\' => match self.take()? {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
                    b'u' => {
                        let unit = self.hex()?;
                        if (0xd800..=0xdbff).contains(&unit) {
                            if self.take()? != b'\\'
                                || self.take()? != b'u'
                                || !(0xdc00..=0xdfff).contains(&self.hex()?)
                            {
                                return Err(Error::Invalid);
                            }
                        } else if (0xdc00..=0xdfff).contains(&unit) {
                            return Err(Error::Invalid);
                        }
                    }
                    _ => return Err(Error::Invalid),
                },
                0..=31 => return Err(Error::Invalid),
                _ => {}
            }
        }
    }
    fn hex(&mut self) -> ResultJson<u16> {
        let mut result = 0;
        for _ in 0..4 {
            result = result * 16 + hex(self.take()?).ok_or(Error::Invalid)?;
        }
        Ok(result)
    }
    fn number(&mut self) -> ResultJson<usize> {
        let start = self.at;
        let negative = self.bytes[self.at] == b'-';
        if negative {
            self.take()?;
        }
        match self.take()? {
            b'0' => {}
            b'1'..=b'9' => {
                while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.take()?;
                }
            }
            _ => return Err(Error::Invalid),
        }
        let mut integer = true;
        if self.bytes.get(self.at) == Some(&b'.') {
            integer = false;
            self.take()?;
            if !self.take()?.is_ascii_digit() {
                return Err(Error::Invalid);
            }
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.take()?;
            }
        }
        if self
            .bytes
            .get(self.at)
            .is_some_and(|byte| matches!(byte, b'e' | b'E'))
        {
            integer = false;
            self.take()?;
            if self
                .bytes
                .get(self.at)
                .is_some_and(|byte| matches!(byte, b'+' | b'-'))
            {
                self.take()?;
            }
            if !self.take()?.is_ascii_digit() {
                return Err(Error::Invalid);
            }
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.take()?;
            }
        }
        let raw = &self.bytes[start..self.at];
        self.ctx.work(raw.len() as u64 + 1)?;
        let magnitude = integer
            .then(|| decimal(raw[usize::from(negative)..].iter().copied(), false))
            .flatten();
        let kind = match (negative, magnitude) {
            (false, Some(value)) => Kind::Unsigned(value),
            (true, Some(value)) if value <= 1 << 63 => {
                Kind::Signed((0u64.wrapping_sub(value)) as i64)
            }
            _ => {
                let number = std::str::from_utf8(raw)
                    .ok()
                    .and_then(|value| value.parse::<f64>().ok())
                    .ok_or(Error::Invalid)?;
                if !number.is_finite() {
                    return Err(Error::Invalid);
                }
                Kind::Real
            }
        };
        self.node(kind, start, self.at)
    }
    fn value(&mut self, depth: usize) -> ResultJson<usize> {
        if depth > MAX_DEPTH {
            return Err(Failure::Limit("Trophy JSON depth").into());
        }
        self.space()?;
        let byte = *self.bytes.get(self.at).ok_or(Error::Invalid)?;
        match byte {
            b'"' => self.text(),
            b'-' | b'0'..=b'9' => self.number(),
            b'n' | b't' | b'f' => {
                let (expected, kind): (&[u8], _) = match byte {
                    b'n' => (b"null", Kind::Null),
                    b't' => (b"true", Kind::Bool(true)),
                    _ => (b"false", Kind::Bool(false)),
                };
                for byte in expected {
                    if self.take()? != *byte {
                        return Err(Error::Invalid);
                    }
                }
                self.node(kind, self.at, self.at)
            }
            b'[' | b'{' => {
                self.take()?;
                let object = byte == b'{';
                let close = if object { b'}' } else { b']' };
                let node = self.node(
                    if object { Kind::Object } else { Kind::Array },
                    self.at,
                    self.at,
                )?;
                self.space()?;
                if self.bytes.get(self.at) == Some(&close) {
                    self.take()?;
                    return Ok(node);
                }
                let mut previous = ABSENT;
                loop {
                    let child = if object {
                        self.text()?
                    } else {
                        self.value(depth + 1)?
                    };
                    if previous == ABSENT {
                        self.nodes[node].child = child;
                    } else {
                        self.nodes[previous].next = child;
                    }
                    previous = child;
                    if object {
                        self.space()?;
                        if self.take()? != b':' {
                            return Err(Error::Invalid);
                        }
                        let value = self.value(depth + 1)?;
                        self.nodes[previous].next = value;
                        previous = value;
                    }
                    self.space()?;
                    match self.take()? {
                        at if at == close => break,
                        b',' => self.space()?,
                        _ => return Err(Error::Invalid),
                    }
                }
                Ok(node)
            }
            _ => Err(Error::Invalid),
        }
    }
}

fn hex(byte: u8) -> Option<u16> {
    match byte {
        b'0'..=b'9' => Some(u16::from(byte - b'0')),
        b'a'..=b'f' => Some(u16::from(byte - b'a' + 10)),
        b'A'..=b'F' => Some(u16::from(byte - b'A' + 10)),
        _ => None,
    }
}

struct StringBytes<'a> {
    bytes: &'a [u8],
    at: usize,
    pending: [u8; 4],
    index: usize,
    count: usize,
}
impl StringBytes<'_> {
    fn unit(&mut self) -> u16 {
        let mut unit = 0;
        for _ in 0..4 {
            unit = unit * 16 + hex(self.bytes[self.at]).expect("validated JSON escape");
            self.at += 1;
        }
        unit
    }
}
impl Iterator for StringBytes<'_> {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        if self.index < self.count {
            let byte = self.pending[self.index];
            self.index += 1;
            return Some(byte);
        }
        let byte = *self.bytes.get(self.at)?;
        self.at += 1;
        if byte != b'\\' {
            return Some(byte);
        }
        let escape = self.bytes[self.at];
        self.at += 1;
        Some(match escape {
            b'b' => 8,
            b'f' => 12,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'u' => {
                let first = self.unit();
                let scalar = if (0xd800..=0xdbff).contains(&first) {
                    self.at += 2;
                    0x10000 + ((u32::from(first) - 0xd800) << 10) + u32::from(self.unit()) - 0xdc00
                } else {
                    u32::from(first)
                };
                let text = char::from_u32(scalar)
                    .expect("validated JSON scalar")
                    .encode_utf8(&mut self.pending);
                self.count = text.len();
                self.index = 1;
                self.pending[0]
            }
            other => other,
        })
    }
}
