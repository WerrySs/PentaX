// SPDX-License-Identifier: GPL-2.0-only
// Adapted from pinned common/trophies.cpp unlock and direct-UDS-rule semantics.

use super::*;

const EPOCH: u64 = 62_135_596_800_000_000;
const LAST_TICK: u64 = 315_537_897_599_999_999;
struct Timestamp {
    id: i32,
    tick: u64,
    key: Vec<u8>,
}

pub(super) fn parse_unlocks(
    bytes: &[u8],
    held: usize,
    budget: &mut Budget,
) -> Result<UnlockData, Failure> {
    let mut ctx = Context::new(held, budget)?;
    if bytes.len() > 1 << 20 {
        return Ok(UnlockData::default());
    }
    let Some(doc) = json::Document::parse(bytes, &mut ctx)? else {
        return Ok(UnlockData::default());
    };
    let Some(unlocked) = field(&doc, doc.root, b"unlockedTrophies", &mut ctx)? else {
        return Ok(UnlockData::default());
    };
    let Some(values) = doc.array(unlocked) else {
        return Ok(UnlockData::default());
    };
    let mut output = UnlockData::default();
    for value in values {
        let id = doc.id(value, &mut ctx)?;
        if id < 0 {
            continue;
        }
        let needed = output.unlocked.len() + 1;
        ctx.reserve(&mut output.unlocked, needed)?;
        output.unlocked.push(id);
    }
    ctx.sort_work(output.unlocked.len())?;
    output.unlocked.sort_unstable();
    output.unlocked.dedup();
    let mut timestamps: Vec<Timestamp> = Vec::new();
    if let Some(at) = field(&doc, doc.root, b"unlockedAt", &mut ctx)?
        && let Some(fields) = doc.object(at)
    {
        for (key, value) in fields {
            if !doc.last_key(key, value, &mut ctx)? {
                continue;
            }
            let id = doc.id(key, &mut ctx)?;
            ctx.work(output.unlocked.len() as u64 + timestamps.len() as u64 + 1)?;
            if id < 0 || output.unlocked.binary_search(&id).is_err() {
                continue;
            }
            let Some(tick) = doc
                .unsigned(value, &mut ctx)?
                .filter(|tick| (EPOCH..=LAST_TICK).contains(tick))
            else {
                continue;
            };
            let name = doc.copy_string(key, &mut ctx)?.expect("object key string");
            if let Some(prior) = timestamps.iter_mut().find(|entry| entry.id == id) {
                // Source JSON object iteration is lexical by decoded key and
                // timestamp map emplace keeps the first numeric-ID alias.
                ctx.work(name.len() as u64 + prior.key.len() as u64 + 1)?;
                if name < prior.key {
                    ctx.replace(&mut prior.key, name);
                    prior.tick = tick;
                } else {
                    ctx.release(name.capacity());
                }
            } else {
                let needed = timestamps.len() + 1;
                ctx.reserve(&mut timestamps, needed)?;
                timestamps.push(Timestamp {
                    id,
                    tick,
                    key: name,
                });
            }
        }
    }
    ctx.sort_work(timestamps.len())?;
    timestamps.sort_unstable_by_key(|entry| entry.id);
    ctx.reserve(&mut output.timestamps, timestamps.len())?;
    ctx.work(timestamps.len() as u64 + 1)?;
    for entry in timestamps {
        output.timestamps.push((entry.id, entry.tick));
    }
    Ok(output)
}

pub(super) fn parse_uds_rules(
    bytes: &[u8],
    held: usize,
    budget: &mut Budget,
) -> Result<Vec<EventRule>, Failure> {
    let mut ctx = Context::new(held, budget)?;
    let Some(files) = ucp::Files::parse(bytes, &mut ctx)? else {
        return Ok(Vec::new());
    };
    let Some(definitions) = files.get(b"stats_definition.json", &mut ctx)? else {
        return Ok(Vec::new());
    };
    let Some(extraction) = files.get(b"stats_extraction.json", &mut ctx)? else {
        return Ok(Vec::new());
    };
    let Some(definitions) = json::Document::parse(definitions, &mut ctx)? else {
        return Ok(Vec::new());
    };
    let Some(extraction) = json::Document::parse(extraction, &mut ctx)? else {
        return Ok(Vec::new());
    };
    let Some(defs) = field(
        &definitions,
        definitions.root,
        b"statDefinitionArray",
        &mut ctx,
    )?
    else {
        return Ok(Vec::new());
    };
    let Some(rules) = field(
        &extraction,
        extraction.root,
        b"statsExtractionRuleArray",
        &mut ctx,
    )?
    else {
        return Ok(Vec::new());
    };
    let Some(defs) = definitions.array(defs) else {
        return Ok(Vec::new());
    };
    let Some(rules) = extraction.array(rules) else {
        return Ok(Vec::new());
    };
    let mut stats: Vec<(u64, bool)> = Vec::new();
    for definition in defs {
        let Some(id) = field(&definitions, definition, b"statId", &mut ctx)? else {
            continue;
        };
        let Some(id) = definitions.unsigned(id, &mut ctx)? else {
            continue;
        };
        let Some(aggregation) = field(&definitions, definition, b"aggregation", &mut ctx)? else {
            continue;
        };
        if !definitions.eq(aggregation, b"latest", &mut ctx)? {
            continue;
        }
        let Some(kind) = field(&definitions, definition, b"dataType", &mut ctx)? else {
            continue;
        };
        let narrow = definitions.eq(kind, b"uint32", &mut ctx)?;
        if !narrow && !definitions.eq(kind, b"uint64", &mut ctx)? {
            continue;
        }
        let maximum = if narrow {
            u64::from(u32::MAX)
        } else {
            u64::MAX
        };
        if let Some(min) = field(&definitions, definition, b"minValue", &mut ctx)?
            && definitions.unsigned(min, &mut ctx)? != Some(0)
        {
            continue;
        }
        if let Some(max) = field(&definitions, definition, b"maxValue", &mut ctx)?
            && definitions.unsigned(max, &mut ctx)? != Some(maximum)
        {
            continue;
        }
        ctx.work(stats.len() as u64 + 1)?;
        if stats.iter().any(|(known, _)| *known == id) {
            continue;
        }
        let needed = stats.len() + 1;
        ctx.reserve(&mut stats, needed)?;
        stats.push((id, narrow));
    }
    let mut events: Vec<EventRule> = Vec::new();
    for rule in rules {
        let Some(condition) = field(&extraction, rule, b"condition", &mut ctx)? else {
            continue;
        };
        let Some(action) = field(&extraction, rule, b"action", &mut ctx)? else {
            continue;
        };
        let Some(event) = extraction.sole_field(condition, b"eventName", &mut ctx)? else {
            continue;
        };
        if !matches!(extraction.kind(event), json::Kind::String) {
            continue;
        }
        let Some(input) = field(&extraction, action, b"input", &mut ctx)? else {
            continue;
        };
        let Some(output) = field(&extraction, action, b"output", &mut ctx)? else {
            continue;
        };
        let Some(id) = field(&extraction, output, b"statId", &mut ctx)? else {
            continue;
        };
        let Some(id) = extraction.unsigned(id, &mut ctx)? else {
            continue;
        };
        ctx.work(stats.len() as u64 + 1)?;
        let Some((_, narrow)) = stats.iter().find(|(known, _)| *known == id) else {
            continue;
        };
        let Some(input) = extraction.copy_string(input, &mut ctx)? else {
            continue;
        };
        ctx.work(input.len() as u64 + 1)?;
        if !input.starts_with(b"$.")
            || input.len() == 2
            || !input[2..]
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            ctx.release(input.capacity());
            continue;
        }
        let property = ctx.copy(&input[2..])?;
        ctx.release(input.capacity());
        drop(input);
        let event = extraction
            .copy_string(event, &mut ctx)?
            .expect("checked string");
        let mut known_event = None;
        for (at, known) in events.iter().enumerate() {
            ctx.work(known.event.len() as u64 + event.len() as u64 + 1)?;
            if known.event == event {
                known_event = Some(at);
                break;
            }
        }
        let at = if let Some(at) = known_event {
            ctx.release(event.capacity());
            drop(event);
            at
        } else {
            let needed = events.len() + 1;
            ctx.reserve(&mut events, needed)?;
            let at = events.len();
            events.push(EventRule {
                event,
                rules: Vec::new(),
            });
            at
        };
        let needed = events[at].rules.len() + 1;
        ctx.reserve(&mut events[at].rules, needed)?;
        events[at].rules.push(UdsRule {
            stat_id: id,
            input: property,
            is_uint32: *narrow,
        });
    }
    ctx.sort_work(events.len())?;
    let key_bytes = events
        .iter()
        .map(|event| event.event.len() as u64)
        .sum::<u64>();
    ctx.work(
        key_bytes
            .checked_mul(events.len() as u64 + 1)
            .ok_or(Failure::Limit("work"))?,
    )?;
    events.sort_unstable_by(|left, right| left.event.cmp(&right.event));
    Ok(events)
}
