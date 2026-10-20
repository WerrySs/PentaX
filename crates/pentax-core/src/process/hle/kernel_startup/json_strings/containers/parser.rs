// SPDX-License-Identifier: GPL-2.0-only
//! Mature JSON lexer with bounded fallible seeds, not an unchecked DOM.
use super::*;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;

struct Context<'a> {
    admission: &'a mut Admission,
    failure: Option<KernelStartupError>,
    negative_zero_integer: Vec<bool>,
    number: usize,
}
struct Seed<'a, 'b> {
    context: &'a mut Context<'b>,
    depth: usize,
}
struct Key<'a, 'b>(&'a mut Context<'b>);

macro_rules! admitted {
    ($context:expr, $operation:expr) => {{
        match $operation {
            Ok(value) => value,
            Err(error) => {
                $context.failure = Some(error);
                return Err(serde::de::Error::custom("bounded JSON admission"));
            }
        }
    }};
}

pub(super) fn parse(
    input: &[u8],
    admission: &mut Admission,
) -> Result<Option<Node>, KernelStartupError> {
    // SliceRead's one escaped-string scratch grows geometrically to at most
    // twice the input; numeric scratch and fixed recursion bookkeeping fit in
    // the other half of this conservative allowance. No serde Value/Map DOM
    // exists: every retained node, entry and text allocation uses our seed.
    // The allowance is reserved BEFORE entering the library and remains held
    // alongside input, output and all other temporary/retained capacities.
    admission.hold(
        input
            .len()
            .checked_mul(4)
            .and_then(|n| n.checked_add(4096))
            .ok_or(KernelStartupError::Budget("JSON parser scratch"))?,
    )?;
    admission.spend(
        input
            .len()
            .checked_mul(64)
            .ok_or(KernelStartupError::Budget("JSON parser work"))?,
    )?;
    let input = input.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(input);
    // The pinned nlohmann lexer treats an unquoted raw NUL as end-of-input.
    // A NUL inside a string remains in the slice and the real lexer rejects it;
    // escaped \\u0000 is decoded normally. Incomplete prefixes still fail.
    let input = source_prefix(input);
    let negative_zero_integer = number_lexemes(input, admission)?;
    let mut context = Context {
        admission,
        failure: None,
        negative_zero_integer,
        number: 0,
    };
    let mut parser = serde_json::Deserializer::from_slice(input);
    let parsed = Seed {
        context: &mut context,
        depth: 0,
    }
    .deserialize(&mut parser);
    match parsed {
        Ok(node) if parser.end().is_ok() => Ok(Some(node)),
        _ => match context.failure {
            Some(error) => Err(error),
            None => Ok(None),
        },
    }
}

fn source_prefix(input: &[u8]) -> &[u8] {
    let mut at = 0;
    while at < input.len() {
        if input[at] == 0 {
            return &input[..at];
        }
        if input[at] == b'"' {
            at += 1;
            while at < input.len() {
                match input[at] {
                    b'\\' => at = (at + 2).min(input.len()),
                    b'"' => {
                        at += 1;
                        break;
                    }
                    _ => at += 1,
                }
            }
        } else {
            at += 1;
        }
    }
    input
}

fn number_lexemes(
    input: &[u8],
    admission: &mut Admission,
) -> Result<Vec<bool>, KernelStartupError> {
    // serde_json preserves lexical -0 as float(-0), whereas nlohmann stores
    // it as signed integer0. Retain only that exact lexical distinction;
    // -0.0 and -0e0 remain genuine floating negative zero. The mature lexer
    // still validates all grammar, escaping and numbers. Values are visited
    // in lexical order, including discarded duplicate object-key values.
    let mut flags = Vec::new();
    let mut at = 0;
    while at < input.len() {
        if input[at] == b'"' {
            at += 1;
            while at < input.len() {
                match input[at] {
                    b'\\' => at = (at + 2).min(input.len()),
                    b'"' => {
                        at += 1;
                        break;
                    }
                    _ => at += 1,
                }
            }
        } else if input[at] == b'-' || input[at].is_ascii_digit() {
            let start = at;
            at += 1;
            while at < input.len()
                && matches!(input[at], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
            {
                at += 1;
            }
            if flags.len() == MAX_NODES {
                return Err(KernelStartupError::Budget("JSON numeric nodes"));
            }
            admission.reserve(&mut flags, 1)?;
            flags.push(&input[start..at] == b"-0");
        } else {
            at += 1;
        }
    }
    Ok(flags)
}
impl Context<'_> {
    fn next_number(&mut self) -> bool {
        let flag = self
            .negative_zero_integer
            .get(self.number)
            .copied()
            .unwrap_or(false);
        self.number += 1;
        flag
    }
}

impl<'de> DeserializeSeed<'de> for Seed<'_, '_> {
    type Value = Node;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        admitted!(self.context, self.context.admission.node(self.depth));
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_, '_> {
    type Value = Node;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Node, E> {
        Ok(Node::Scalar(0, 0))
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Node, E> {
        Ok(Node::Scalar(1, u64::from(value)))
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Node, E> {
        self.context.next_number();
        Ok(Node::Scalar(2, value as u64))
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Node, E> {
        self.context.next_number();
        Ok(Node::Scalar(3, value))
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Node, E> {
        Ok(if self.context.next_number() {
            Node::Scalar(2, 0)
        } else {
            Node::Scalar(4, value.to_bits())
        })
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Node, E> {
        Ok(Node::String(admitted!(
            self.context,
            self.context.admission.bytes(value.as_bytes())
        )))
    }
    fn visit_borrowed_str<E: serde::de::Error>(self, value: &'de str) -> Result<Node, E> {
        self.visit_str(value)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut array: A) -> Result<Node, A::Error> {
        let mut children = Vec::new();
        while let Some(child) = array.next_element_seed(Seed {
            context: &mut *self.context,
            depth: self.depth + 1,
        })? {
            admitted!(
                self.context,
                self.context.admission.reserve(&mut children, 1)
            );
            children.push(child);
        }
        Ok(Node::Array(children))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Node, A::Error> {
        let mut children: Vec<(Vec<u8>, Node)> = Vec::new();
        while let Some(key) = object.next_key_seed(Key(&mut *self.context))? {
            let value = object.next_value_seed(Seed {
                context: &mut *self.context,
                depth: self.depth + 1,
            })?;
            admitted!(
                self.context,
                self.context.admission.spend(children.len() + key.len())
            );
            match children.binary_search_by(|entry| entry.0.cmp(&key)) {
                Ok(index) => children[index].1 = value, // Source's default JSON map: last key wins.
                Err(index) => {
                    admitted!(
                        self.context,
                        self.context.admission.reserve(&mut children, 1)
                    );
                    children.insert(index, (key, value));
                }
            }
        }
        Ok(Node::Object(children))
    }
}
impl<'de> DeserializeSeed<'de> for Key<'_, '_> {
    type Value = Vec<u8>;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        deserializer.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key<'_, '_> {
    type Value = Vec<u8>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON object key")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Vec<u8>, E> {
        Ok(admitted!(self.0, self.0.admission.bytes(value.as_bytes())))
    }
    fn visit_borrowed_str<E: serde::de::Error>(self, value: &'de str) -> Result<Vec<u8>, E> {
        self.visit_str(value)
    }
}
