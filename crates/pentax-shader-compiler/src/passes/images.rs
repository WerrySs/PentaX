// SPDX-License-Identifier: GPL-2.0-only
// Original finite tracking for direct image/sampler user-data descriptors.
use super::*;
use crate::profile::MAX_IMAGES;

#[derive(Clone, Copy, Debug)]
pub(crate) struct LodCounterUse {
    pub image: usize,
    pub profile_index: usize,
    pub first_use_pc: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SamplerUse {
    pub user_sgpr: u8,
    pub image_user_sgpr: u8,
    pub profile_index: usize,
    pub first_use_pc: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ImageUse {
    pub user_sgpr: u8,
    pub profile_index: usize,
    pub first_use_pc: u32,
}

pub(crate) fn discover_images(
    ir: &Ir,
    budget: &mut Budget,
) -> Result<(Vec<crate::ImageRequirement>, Vec<crate::SamplerRequirement>), CompileError> {
    let mut images: Vec<crate::ImageRequirement> = Vec::new();
    let mut samplers: Vec<crate::SamplerRequirement> = Vec::new();
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            budget.charge(1)?;
            let inst = ir.inst(id);
            if !inst.op.is_image() {
                continue;
            }
            for (argument, op, width, entries) in [
                (0, Op::GetImageResource, 8, &mut images),
                (1, Op::GetSamplerResource, 4, &mut samplers),
            ] {
                let base = descriptor_base(ir, budget, ir.arg(id, argument), inst.pc, op, width)?;
                budget.charge_usize(entries.len())?;
                if let Some(entry) = entries.iter_mut().find(|entry| entry.user_sgpr == base) {
                    entry.first_use_pc = entry.first_use_pc.min(inst.pc);
                } else {
                    budget.push(
                        entries,
                        crate::ImageRequirement {
                            user_sgpr: base,
                            first_use_pc: inst.pc,
                        },
                        MAX_IMAGES,
                        LimitKind::SourceTable,
                    )?;
                }
            }
        }
    }
    Ok((images, samplers))
}

pub(crate) fn track_images(
    ir: &mut Ir,
    budget: &mut Budget,
    profile: &Profile,
    layout: &mut Layout,
) -> Result<(), CompileError> {
    let (images, samplers) = discover_images(ir, budget)?;
    let mut sampler_uses = Vec::new();
    for (sampler, requirements, entries) in [
        (false, &images, &mut layout.images),
        (true, &samplers, &mut sampler_uses),
    ] {
        for requirement in requirements {
            let base = requirement.user_sgpr;
            let index = if sampler {
                budget.charge_usize(profile.common().samplers.len())?;
                profile
                    .common()
                    .samplers
                    .iter()
                    .position(|d| d.user_sgpr == base)
            } else {
                budget.charge_usize(profile.common().images.len())?;
                profile
                    .common()
                    .images
                    .iter()
                    .position(|d| d.user_sgpr == base)
            }
            .ok_or_else(|| {
                CompileError::resource(
                    Some(requirement.first_use_pc),
                    if sampler {
                        ResourceErrorKind::MissingSampler { user_sgpr: base }
                    } else {
                        ResourceErrorKind::MissingImage { user_sgpr: base }
                    },
                )
            })?;
            budget.push(
                entries,
                ImageUse {
                    user_sgpr: base,
                    profile_index: index,
                    first_use_pc: requirement.first_use_pc,
                },
                MAX_IMAGES,
                LimitKind::SourceTable,
            )?;
        }
        let count = if sampler {
            profile.common().samplers.len()
        } else {
            profile.common().images.len()
        };
        if count != entries.len() {
            budget.charge_usize(count * entries.len().max(1))?;
            let unused = if sampler {
                profile
                    .common()
                    .samplers
                    .iter()
                    .map(|d| d.user_sgpr)
                    .find(|base| !entries.iter().any(|entry| entry.user_sgpr == *base))
            } else {
                profile
                    .common()
                    .images
                    .iter()
                    .map(|d| d.user_sgpr)
                    .find(|base| !entries.iter().any(|entry| entry.user_sgpr == *base))
            }
            .ok_or(CompileError::Internal("image profile cardinality"))?;
            return Err(CompileError::resource(
                None,
                if sampler {
                    ResourceErrorKind::UnusedSampler { user_sgpr: unused }
                } else {
                    ResourceErrorKind::UnusedImage { user_sgpr: unused }
                },
            ));
        }
    }
    for block in 0..ir.blocks.len() as u32 {
        let ids = ssa::collect_block(ir, budget, block)?;
        for id in ids {
            budget.charge(1)?;
            let inst = *ir.inst(id);
            if !inst.op.is_image() {
                continue;
            }
            let image =
                descriptor_base(ir, budget, ir.arg(id, 0), inst.pc, Op::GetImageResource, 8)?;
            let sampler = descriptor_base(
                ir,
                budget,
                ir.arg(id, 1),
                inst.pc,
                Op::GetSamplerResource,
                4,
            )?;
            budget
                .charge_usize(layout.images.len() + sampler_uses.len() + layout.samplers.len())?;
            let image_user_sgpr = image;
            let image = layout
                .images
                .iter()
                .position(|d| d.user_sgpr == image)
                .ok_or(CompileError::Internal("tracked image missing"))?;
            let source = sampler_uses
                .iter()
                .find(|d| d.user_sgpr == sampler)
                .ok_or(CompileError::Internal("tracked sampler missing"))?;
            let sampler = if let Some(index) = layout.samplers.iter().position(|pair| {
                pair.user_sgpr == sampler && pair.image_user_sgpr == image_user_sgpr
            }) {
                layout.samplers[index].first_use_pc =
                    layout.samplers[index].first_use_pc.min(inst.pc);
                index
            } else {
                budget.push(
                    &mut layout.samplers,
                    SamplerUse {
                        user_sgpr: source.user_sgpr,
                        image_user_sgpr,
                        profile_index: source.profile_index,
                        first_use_pc: inst.pc,
                    },
                    MAX_IMAGES,
                    LimitKind::SourceTable,
                )?;
                layout.samplers.len() - 1
            };
            ir.inst_mut(id).aux = image as u32 | ((sampler as u32) << 16);
            if inst.op.has_lod_feedback() {
                budget.charge_usize(profile.common().lod_counters.len())?;
                if let Some(profile_index) = profile
                    .common()
                    .lod_counters
                    .iter()
                    .position(|counter| counter.user_sgpr == layout.images[image].user_sgpr)
                {
                    if profile.common().subgroup.is_none() {
                        return Err(CompileError::unsupported(
                            inst.pc,
                            None,
                            UnsupportedReason::RequiresExactSubgroup,
                        ));
                    }
                    budget.charge_usize(layout.lod_counters.len())?;
                    if let Some(entry) = layout
                        .lod_counters
                        .iter_mut()
                        .find(|entry| entry.image == image)
                    {
                        entry.first_use_pc = entry.first_use_pc.min(inst.pc);
                    } else {
                        budget.push(
                            &mut layout.lod_counters,
                            LodCounterUse {
                                image,
                                profile_index,
                                first_use_pc: inst.pc,
                            },
                            MAX_IMAGES,
                            LimitKind::SourceTable,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}
