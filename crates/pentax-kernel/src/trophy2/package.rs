// SPDX-License-Identifier: GPL-2.0-only
// Adapted from common/trophies.cpp/h, authorized KytyPS5-Performance source
// 209dffe9032d72749257ce18680d506b891284de. Source blobs and limits are in
// docs/upstream-provenance.md and NOTICE. No SDK/firmware data.

use crate::Failure;
use crate::budget::Budget;

mod json;
mod saves_rules;
#[cfg(test)]
mod tests;
mod ucp;

const MAX_METADATA: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Comparison {
    #[default]
    None,
    GreaterEqual,
    Greater,
    LessEqual,
    Less,
}
pub(super) struct Group {
    pub id: i32,
    pub title: Vec<u8>,
}
pub(super) struct Trophy {
    pub id: i32,
    pub group_id: i32,
    // Retained source metadata for the separate UDS evaluation lifecycle.
    pub platinum_id: i32,
    pub grade: i32,
    pub target: Option<u64>,
    pub uds_stat_id: Option<u64>,
    pub comparison: Comparison,
    pub progressive: bool,
    pub name: Vec<u8>,
    pub description: Vec<u8>,
    pub reward: Vec<u8>,
    pub icon_png: Vec<u8>,
    pub hidden: bool,
    pub has_reward: bool,
}
pub(super) struct UdsRule {
    // Source rule data used by direct latest-value UDS evaluation.
    pub stat_id: u64,
    pub input: Vec<u8>,
    pub is_uint32: bool,
}
pub(super) struct EventRule {
    pub event: Vec<u8>,
    pub rules: Vec<UdsRule>,
}
pub(super) struct Package {
    pub title: Vec<u8>,
    pub groups: Vec<Group>,
    pub trophies: Vec<Trophy>,
    pub event_rules: Vec<EventRule>,
}
#[derive(Default)]
pub(super) struct UnlockData {
    pub unlocked: Vec<i32>,
    pub timestamps: Vec<(i32, u64)>,
}

fn capacity<T>(values: &Vec<T>) -> usize {
    values.capacity() * core::mem::size_of::<T>()
}
pub(super) fn rules_metadata_bytes(rules: &Vec<EventRule>) -> usize {
    capacity(rules)
        + rules
            .iter()
            .map(|event| {
                event.event.capacity()
                    + capacity(&event.rules)
                    + event
                        .rules
                        .iter()
                        .map(|rule| rule.input.capacity())
                        .sum::<usize>()
            })
            .sum::<usize>()
}
impl Package {
    pub fn metadata_bytes(&self) -> usize {
        self.title.capacity()
            + capacity(&self.groups)
            + capacity(&self.trophies)
            + self
                .groups
                .iter()
                .map(|group| group.title.capacity())
                .sum::<usize>()
            + self
                .trophies
                .iter()
                .map(|trophy| {
                    trophy.name.capacity()
                        + trophy.description.capacity()
                        + trophy.reward.capacity()
                        + trophy.icon_png.capacity()
                })
                .sum::<usize>()
            + rules_metadata_bytes(&self.event_rules)
    }
}
impl UnlockData {
    pub fn metadata_bytes(&self) -> usize {
        capacity(&self.unlocked) + capacity(&self.timestamps)
    }
}

pub(super) fn parse_unlocks(
    bytes: &[u8],
    held: usize,
    budget: &mut Budget,
) -> Result<UnlockData, Failure> {
    saves_rules::parse_unlocks(bytes, held, budget)
}
pub(super) fn parse_uds_rules(
    bytes: &[u8],
    held: usize,
    budget: &mut Budget,
) -> Result<Vec<EventRule>, Failure> {
    saves_rules::parse_uds_rules(bytes, held, budget)
}

/// The caller's actual input capacity and retained staging are already live.
/// Every new capacity is admitted alongside old storage before replacement.
struct Context<'a> {
    budget: &'a mut Budget,
    live: usize,
}
impl<'a> Context<'a> {
    fn new(held: usize, budget: &'a mut Budget) -> Result<Self, Failure> {
        if held > MAX_METADATA {
            return Err(Failure::Limit("Trophy parser metadata"));
        }
        budget.scratch(held)?;
        budget.record_scratch(held);
        Ok(Self { budget, live: held })
    }
    fn work(&mut self, amount: u64) -> Result<(), Failure> {
        self.budget.work(amount)
    }
    fn release(&mut self, bytes: usize) {
        self.live = self
            .live
            .checked_sub(bytes)
            .expect("owned parser capacities");
    }
    fn reserve<T>(&mut self, values: &mut Vec<T>, needed: usize) -> Result<(), Failure> {
        if needed <= values.capacity() {
            return Ok(());
        }
        let target = needed.max(values.capacity().saturating_mul(2)).max(1);
        let requested = target
            .checked_mul(core::mem::size_of::<T>())
            .ok_or(Failure::Limit("Trophy parser metadata"))?;
        self.admit(requested)?;
        self.work(
            (requested as u64)
                .checked_add(capacity(values) as u64)
                .ok_or(Failure::Limit("work"))?,
        )?;
        let mut next = Vec::new();
        next.try_reserve_exact(target)
            .map_err(|_| Failure::Allocation)?;
        let actual = capacity(&next);
        self.admit(actual)?;
        let old = capacity(values);
        next.append(values);
        *values = next;
        self.live = self.live + actual - old;
        Ok(())
    }
    fn admit(&mut self, extra: usize) -> Result<(), Failure> {
        let total = self
            .live
            .checked_add(extra)
            .filter(|total| *total <= MAX_METADATA)
            .ok_or(Failure::Limit("Trophy parser metadata"))?;
        self.budget.scratch(total)?;
        self.budget.record_scratch(total);
        Ok(())
    }
    fn copy(&mut self, bytes: &[u8]) -> Result<Vec<u8>, Failure> {
        let mut output = Vec::new();
        self.reserve(&mut output, bytes.len())?;
        self.work(bytes.len() as u64 + 1)?;
        output.extend_from_slice(bytes);
        Ok(output)
    }
    fn replace(&mut self, target: &mut Vec<u8>, value: Vec<u8>) {
        self.release(target.capacity());
        *target = value;
    }
    fn sort_work(&mut self, count: usize) -> Result<(), Failure> {
        self.work(
            (count as u64)
                .checked_mul(u64::from(count.max(1).ilog2()) + 1)
                .and_then(|work| work.checked_mul(64))
                .ok_or(Failure::Limit("work"))?,
        )
    }
}

fn field(
    doc: &json::Document<'_>,
    node: usize,
    name: &[u8],
    ctx: &mut Context<'_>,
) -> Result<Option<usize>, Failure> {
    doc.field(node, name, ctx)
}
fn boolean(
    doc: &json::Document<'_>,
    node: usize,
    name: &[u8],
    ctx: &mut Context<'_>,
) -> Result<bool, Failure> {
    Ok(field(doc, node, name, ctx)?
        .is_some_and(|at| matches!(doc.kind(at), json::Kind::Bool(true))))
}

const LOCALES: [&[u8]; 30] = [
    b"ja-JP", b"en-US", b"fr-FR", b"es-ES", b"de-DE", b"it-IT", b"nl-NL", b"pt-PT", b"ru-RU",
    b"ko-KR", b"zh-Hant", b"zh-Hans", b"fi-FI", b"sv-SE", b"da-DK", b"no-NO", b"pl-PL", b"pt-BR",
    b"en-GB", b"tr-TR", b"es-419", b"ar-AE", b"fr-CA", b"cs-CZ", b"hu-HU", b"el-GR", b"ro-RO",
    b"th-TH", b"vi-VN", b"id-ID",
];

pub(super) fn parse_package(
    bytes: &[u8],
    language: u8,
    held: usize,
    budget: &mut Budget,
) -> Result<Option<Package>, Failure> {
    let mut ctx = Context::new(held, budget)?;
    let Some(files) = ucp::Files::parse(bytes, &mut ctx)? else {
        return Ok(None);
    };
    let Some(raw) = files.get(b"tropconf.json", &mut ctx)? else {
        return Ok(None);
    };
    let Some(conf) = json::Document::parse(raw, &mut ctx)? else {
        return Ok(None);
    };
    let Some(trophies_node) = field(&conf, conf.root, b"trophies", &mut ctx)? else {
        return Ok(None);
    };
    let Some(definitions) = conf.array(trophies_node) else {
        return Ok(None);
    };
    let count = definitions.count();
    ctx.work(count as u64 + 1)?;
    if count > 1000 {
        return Ok(None);
    }
    let Some(default) = field(&conf, conf.root, b"defaultLanguage", &mut ctx)? else {
        return Ok(None);
    };
    if !matches!(conf.kind(default), json::Kind::String) {
        return Ok(None);
    }
    let locale = LOCALES
        .get(usize::from(language))
        .copied()
        .unwrap_or(b"en-US");
    let selected = files.localized(locale, &mut ctx)?;
    let mut meta = match selected {
        Some(raw) => {
            let Some(doc) = json::Document::parse(raw, &mut ctx)? else {
                return Ok(None);
            };
            Some(doc)
        }
        None => None,
    };
    if meta
        .as_ref()
        .is_none_or(|doc| matches!(doc.kind(doc.root), json::Kind::Null))
    {
        if let Some(doc) = meta.take() {
            ctx.release(doc.metadata_bytes());
        }
        let locale = conf
            .copy_string(default, &mut ctx)?
            .expect("checked string");
        let selected = files.localized(&locale, &mut ctx)?;
        ctx.release(locale.capacity());
        drop(locale);
        let Some(raw) = selected else {
            return Ok(None);
        };
        meta = json::Document::parse(raw, &mut ctx)?;
    }
    let Some(meta) = meta else {
        return Ok(None);
    };
    let Some(texts) = field(&meta, meta.root, b"metadata", &mut ctx)? else {
        return Ok(None);
    };
    let Some(title_node) = field(&meta, texts, b"titleMetadata", &mut ctx)? else {
        return Ok(None);
    };
    let Some(name) = field(&meta, title_node, b"name", &mut ctx)? else {
        return Ok(None);
    };
    let Some(title) = meta.copy_string(name, &mut ctx)? else {
        return Ok(None);
    };
    let Some(trophy_texts) = field(&meta, texts, b"trophyMetadata", &mut ctx)? else {
        return Ok(None);
    };
    if meta.array(trophy_texts).is_none() {
        return Ok(None);
    }
    let mut package = Package {
        title,
        groups: Vec::new(),
        trophies: Vec::new(),
        event_rules: Vec::new(),
    };
    ctx.reserve(&mut package.groups, 1)?;
    package.groups.push(Group {
        id: -1,
        title: ctx.copy(&package.title)?,
    });
    let mut icon_bytes = 0usize;
    for definition in conf.array(trophies_node).expect("checked array") {
        let Some(id) = field(&conf, definition, b"id", &mut ctx)? else {
            return Ok(None);
        };
        let id = conf.id(id, &mut ctx)?;
        let Some(grade_node) = field(&conf, definition, b"grade", &mut ctx)? else {
            return Ok(None);
        };
        let grade = if conf.eq(grade_node, b"P", &mut ctx)? {
            1
        } else if conf.eq(grade_node, b"G", &mut ctx)? {
            2
        } else if conf.eq(grade_node, b"S", &mut ctx)? {
            3
        } else if conf.eq(grade_node, b"B", &mut ctx)? {
            4
        } else {
            return Ok(None);
        };
        ctx.work(package.trophies.len() as u64 + 1)?;
        if id < 0 || package.trophies.iter().any(|trophy| trophy.id == id) {
            return Ok(None);
        }
        let group_id = if let Some(group) = field(&conf, definition, b"groupId", &mut ctx)? {
            let id = conf.id(group, &mut ctx)?;
            if id < 0 {
                return Ok(None);
            }
            id
        } else {
            -1
        };
        let platinum_id =
            if let Some(platinum) = field(&conf, definition, b"platinumTrophyId", &mut ctx)? {
                conf.id(platinum, &mut ctx)?
            } else {
                -1
            };
        let mut trophy = Trophy {
            id,
            group_id,
            platinum_id,
            grade,
            target: None,
            uds_stat_id: None,
            comparison: Comparison::None,
            progressive: false,
            name: Vec::new(),
            description: Vec::new(),
            reward: Vec::new(),
            icon_png: Vec::new(),
            hidden: boolean(&conf, definition, b"hidden", &mut ctx)?,
            has_reward: boolean(&conf, definition, b"hasReward", &mut ctx)?,
        };
        if let Some(condition) = field(&conf, definition, b"unlockCondition", &mut ctx)?
            && matches!(conf.kind(condition), json::Kind::Object)
        {
            trophy.progressive = boolean(&conf, condition, b"progressive", &mut ctx)?;
            if let Some(stat) = field(&conf, condition, b"udsStatId", &mut ctx)? {
                trophy.uds_stat_id = conf.unsigned(stat, &mut ctx)?;
                if trophy.uds_stat_id.is_none() {
                    return Ok(None);
                }
            }
            if let Some(target) = field(&conf, condition, b"targetValue", &mut ctx)? {
                trophy.target = conf.unsigned(target, &mut ctx)?;
            }
            if let Some(comparator) = field(&conf, condition, b"comparator", &mut ctx)? {
                for (text, comparison) in [
                    (b"ge", Comparison::GreaterEqual),
                    (b"gt", Comparison::Greater),
                    (b"le", Comparison::LessEqual),
                    (b"lt", Comparison::Less),
                ] {
                    if conf.eq(comparator, text, &mut ctx)? {
                        trophy.comparison = comparison;
                        break;
                    }
                }
            }
        }
        let mut icon = [0; 18];
        let icon_length = icon_name(id, &mut icon);
        if let Some(bytes) = files.get(&icon[..icon_length], &mut ctx)? {
            if bytes.len() > files.size - icon_bytes {
                return Ok(None);
            }
            trophy.icon_png = ctx.copy(bytes)?;
            icon_bytes += bytes.len();
        }
        ctx.work(package.groups.len() as u64 + 1)?;
        if !package.groups.iter().any(|group| group.id == group_id) {
            if package.groups.len() == 50 {
                return Ok(None);
            }
            let needed = package.groups.len() + 1;
            ctx.reserve(&mut package.groups, needed)?;
            package.groups.push(Group {
                id: group_id,
                title: Vec::new(),
            });
        }
        let needed = package.trophies.len() + 1;
        ctx.reserve(&mut package.trophies, needed)?;
        package.trophies.push(trophy);
    }
    if let Some(groups) = field(&meta, texts, b"groupMetadata", &mut ctx)?
        && let Some(groups) = meta.array(groups)
    {
        for group in groups {
            let Some(id) = field(&meta, group, b"id", &mut ctx)? else {
                continue;
            };
            let Some(name) = field(&meta, group, b"name", &mut ctx)? else {
                continue;
            };
            let id = meta.id(id, &mut ctx)?;
            ctx.work(package.groups.len() as u64 + 1)?;
            if let Some(group) = package.groups.iter_mut().find(|group| group.id == id)
                && let Some(name) = meta.copy_string(name, &mut ctx)?
            {
                ctx.replace(&mut group.title, name);
            }
        }
    }
    for text in meta.array(trophy_texts).expect("checked array") {
        let Some(id) = field(&meta, text, b"id", &mut ctx)? else {
            continue;
        };
        let id = meta.id(id, &mut ctx)?;
        ctx.work(package.trophies.len() as u64 + 1)?;
        if let Some(trophy) = package.trophies.iter_mut().find(|trophy| trophy.id == id) {
            for (key, target) in [
                (b"name".as_slice(), &mut trophy.name),
                (b"detail".as_slice(), &mut trophy.description),
                (b"reward".as_slice(), &mut trophy.reward),
            ] {
                if let Some(node) = field(&meta, text, key, &mut ctx)?
                    && let Some(value) = meta.copy_string(node, &mut ctx)?
                {
                    ctx.replace(target, value);
                }
            }
        }
    }
    ctx.sort_work(package.groups.len() + package.trophies.len())?;
    package.groups.sort_unstable_by_key(|group| group.id);
    package.trophies.sort_unstable_by_key(|trophy| trophy.id);
    Ok(Some(package))
}

fn icon_name(id: i32, output: &mut [u8; 18]) -> usize {
    output[..4].copy_from_slice(b"trop");
    let mut digits = [b'0'; 10];
    let mut count = 0;
    let mut number = id as u32;
    loop {
        digits[count] = b'0' + (number % 10) as u8;
        count += 1;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    let width = count.max(4);
    output[4..4 + width].fill(b'0');
    for (index, digit) in digits[..count].iter().enumerate() {
        output[4 + width - index - 1] = *digit;
    }
    output[4 + width..8 + width].copy_from_slice(b".png");
    8 + width
}
