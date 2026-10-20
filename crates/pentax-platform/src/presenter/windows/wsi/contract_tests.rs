// SPDX-License-Identifier: GPL-2.0-only
//! Original capability records only; no loader, window, GPU or clock is opened.
use super::*;

fn extent() -> vk::Extent2D {
    vk::Extent2D {
        width: 64,
        height: 64,
    }
}

fn capabilities() -> vk::SurfaceCapabilitiesKHR {
    vk::SurfaceCapabilitiesKHR {
        min_image_count: 2,
        max_image_count: 5,
        current_extent: extent(),
        min_image_extent: vk::Extent2D {
            width: 16,
            height: 16,
        },
        max_image_extent: vk::Extent2D {
            width: 128,
            height: 128,
        },
        max_image_array_layers: 2,
        supported_transforms: vk::SurfaceTransformFlagsKHR::IDENTITY
            | vk::SurfaceTransformFlagsKHR::ROTATE_90,
        current_transform: vk::SurfaceTransformFlagsKHR::IDENTITY,
        supported_composite_alpha: vk::CompositeAlphaFlagsKHR::OPAQUE
            | vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
        supported_usage_flags: vk::ImageUsageFlags::TRANSFER_DST
            | vk::ImageUsageFlags::TRANSFER_SRC
            | vk::ImageUsageFlags::COLOR_ATTACHMENT,
    }
}

#[test]
fn selected_surface_allows_only_changes_that_keep_every_selected_requirement() {
    let mut caps = capabilities();
    assert_eq!(surface_contract(caps, extent(), 3), Ok(()));
    // The actual request stays three images, one layer, identity transform,
    // opaque alpha and transfer destination. Unused advertised choices change.
    caps.min_image_count = 1;
    caps.max_image_count = 0; // No advertised upper count limit.
    caps.min_image_extent = vk::Extent2D {
        width: 32,
        height: 32,
    };
    caps.max_image_extent = vk::Extent2D {
        width: 256,
        height: 256,
    };
    caps.max_image_array_layers = 4;
    caps.supported_transforms =
        vk::SurfaceTransformFlagsKHR::IDENTITY | vk::SurfaceTransformFlagsKHR::ROTATE_180;
    caps.supported_composite_alpha =
        vk::CompositeAlphaFlagsKHR::OPAQUE | vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED;
    caps.supported_usage_flags = vk::ImageUsageFlags::TRANSFER_DST
        | vk::ImageUsageFlags::COLOR_ATTACHMENT
        | vk::ImageUsageFlags::SAMPLED;
    assert_eq!(surface_contract(caps, extent(), 3), Ok(()));
    caps.current_extent = vk::Extent2D {
        width: u32::MAX,
        height: u32::MAX,
    };
    assert_eq!(surface_contract(caps, extent(), 3), Ok(()));
    assert_eq!(returned_image_count(3, 3), Ok(3));
    assert_eq!(returned_image_count(8, 3), Ok(8));
}

#[test]
fn selected_surface_refuses_each_required_loss_and_invalid_returned_count() {
    use WindowChange::*;
    for (case, expected) in [
        (0, SurfaceImageCount),
        (1, SurfaceImageCount),
        (2, SurfaceExtent),
        (3, SurfaceExtent),
        (4, SurfaceLimits),
        (5, SurfaceLimits),
        (6, SurfaceLimits),
        (7, SurfaceLimits),
        (8, SurfaceLimits),
        (9, SurfaceProfile),
        (10, SurfaceProfile),
        (11, SurfaceProfile),
        (12, SurfaceProfile),
    ] {
        let mut caps = capabilities();
        match case {
            0 => caps.min_image_count = 4,
            1 => caps.max_image_count = 2,
            2 => caps.current_extent.width = 63,
            3 => caps.current_extent.width = u32::MAX, // Half undefined is not the sentinel.
            4 => caps.min_image_extent.width = 65,
            5 => caps.min_image_extent.height = 65,
            6 => caps.max_image_extent.width = 63,
            7 => caps.max_image_extent.height = 63,
            8 => caps.max_image_array_layers = 0,
            9 => caps.supported_transforms = vk::SurfaceTransformFlagsKHR::ROTATE_90,
            10 => caps.current_transform = vk::SurfaceTransformFlagsKHR::ROTATE_90,
            11 => caps.supported_composite_alpha = vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            12 => caps.supported_usage_flags = vk::ImageUsageFlags::COLOR_ATTACHMENT,
            _ => unreachable!(),
        }
        assert_eq!(surface_contract(caps, extent(), 3), Err(expected), "{case}");
    }
    for requested in [0, 9] {
        assert_eq!(
            surface_contract(capabilities(), extent(), requested),
            Err(SurfaceImageCount)
        );
    }
    for empty in [
        vk::Extent2D {
            width: 0,
            height: 64,
        },
        vk::Extent2D {
            width: 64,
            height: 0,
        },
    ] {
        assert_eq!(
            surface_contract(capabilities(), empty, 3),
            Err(SurfaceExtent)
        );
    }
    for (actual, requested) in [(0, 3), (2, 3), (9, 3), (3, 0), (8, 9)] {
        assert_eq!(
            returned_image_count(actual, requested),
            Err(Error::UnavailableCompletion)
        );
    }
}
