// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from public KytyPS5 7b7f9a70 agc.cpp four CopyData/DmaData producers; see NOTICE.
//! Exact encoders only. No encoded transfer address is dereferenced here.
use super::{Function, Packet, pm4};

pub(super) fn packet(function: Function, args: [u64; 12]) -> Option<Packet> {
    use Function::*;
    if matches!(function, AgcAcbCopyData | AgcDcbCopyData) {
        let src = u32::from(args[4] as u8);
        let dst = u32::from(args[1] as u8);
        let selectors = if function == AgcAcbCopyData {
            src & 15 | (dst & 15) << 8
        } else {
            (src >> 1 & 15) | (dst >> 1 & 15) << 8 | (src & 1) << 30
        };
        return Some(Packet::fixed(&[
            pm4(6, 0x40, 0),
            selectors
                | (u32::from(args[5] as u8) & 3) << 13
                | (u32::from(args[7] as u8) & 1) << 16
                | (u32::from(args[8] as u8) & 1) << 20
                | (u32::from(args[2] as u8) & 3) << 25,
            args[6] as u32,
            (args[6] >> 32) as u32,
            args[3] as u32,
            (args[3] >> 32) as u32,
        ]));
    }
    if !matches!(function, AgcAcbDmaData | AgcDcbDmaData) {
        return None;
    }
    let (
        engine,
        dst,
        dst_cache,
        dst_address,
        src,
        src_cache,
        mut source,
        count,
        wait,
        confirm,
        block,
    ) = if function == AgcAcbDmaData {
        (
            0,
            args[1] as u8,
            args[2] as u8,
            args[3],
            args[4] as u8,
            args[5] as u8,
            args[6],
            args[7] as u32,
            args[8] as u8,
            args[9] as u8,
            0,
        )
    } else {
        (
            args[1] as u8,
            args[2] as u8,
            args[3] as u8,
            args[4],
            args[5] as u8,
            args[6] as u8,
            args[7],
            args[8] as u32,
            args[9] as u8,
            args[10] as u8,
            args[11] as u8,
        )
    };
    // The remap compares the full narrowed engine byte before packet bit masking.
    source = match src {
        0x14 => {
            if function == AgcDcbDmaData && engine == 1 {
                0x30148
            } else {
                0x30174
            }
        }
        0x24 => {
            if function == AgcDcbDmaData && engine == 1 {
                0x30150
            } else {
                0x3017c
            }
        }
        0x25 if function == AgcAcbDmaData => 0x30184,
        0x64 if function == AgcDcbDmaData => {
            if engine == 1 {
                0x30158
            } else {
                0x30184
            }
        }
        _ => source,
    };
    let control = u32::from(engine & 1)
        | u32::from(src_cache & 3) << 13
        | u32::from(dst & 3) << 20
        | u32::from(dst_cache & 3) << 25
        | u32::from(src & 3) << 29
        | u32::from(block & 1) << 31;
    let control2 = count & 0x03ff_ffff
        | u32::from(src & 4) << 24
        | u32::from(dst & 4) << 25
        | u32::from(src & 8) << 25
        | u32::from(dst & 8) << 26
        | u32::from(wait & 1) << 30
        | u32::from(confirm & 1) << 31;
    Some(Packet::fixed(&[
        pm4(7, 0x50, 0),
        control,
        source as u32,
        (source >> 32) as u32,
        dst_address as u32,
        (dst_address >> 32) as u32,
        control2,
    ]))
}
