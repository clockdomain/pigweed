// Copyright 2026 The Pigweed Authors
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not
// use this file except in compliance with the License. You may obtain a copy of
// the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
// License for the specific language governing permissions and limitations under
// the License.

//! PMSAv7 (ARMv7-M) MPU validation.
//!
//! PMSAv7 has strict power-of-2 alignment requirements:
//! - Region sizes must be power-of-2 (32 bytes to 4GB)
//! - Region base must be aligned to region size
//! - 8 subregions per region (each 1/8 of total size)
//!
//! These constraints can cause MPU regions to "bloat" beyond their requested
//! size, potentially overlapping with kernel memory.

use super::{MemoryRegion, MpuIssue};

/// Result of PMSAv7 region calculation.
#[derive(Clone, Debug)]
pub struct Pmsav7Region {
    /// Aligned base address
    pub base: u64,
    /// Region size (power of 2)
    pub size: u64,
    /// SIZE field value for RASR register (log2(size) - 1)
    pub size_field: u32,
    /// Subregion size (size / 8)
    pub subregion_size: u64,
    /// Subregion disable mask (SRD)
    pub srd_mask: u8,
    /// Indices of enabled subregions (0-7)
    pub enabled_subregions: Vec<u8>,
}

/// Calculate the PMSAv7 aligned region for a memory range.
///
/// PMSAv7 requires:
/// - Power-of-2 region sizes (32 bytes to 4GB)
/// - Region base aligned to region size
/// - 8 subregions per region (each 1/8 of total size)
#[must_use]
pub fn calculate_pmsav7_region(start: u64, end: u64) -> Pmsav7Region {
    let requested_size = end - start;

    // Find smallest power-of-2 region size that covers the range
    let mut region_size: u64 = 32; // Minimum 32 bytes
    while region_size < requested_size {
        region_size *= 2;
    }

    // Align base to region size
    let mut aligned_base = start & !(region_size - 1);

    // Check if aligned region covers the end address
    while aligned_base + region_size < end {
        region_size *= 2;
        aligned_base = start & !(region_size - 1);
    }

    // Calculate SIZE field: log2(region_size) - 1
    let size_field = (region_size.trailing_zeros()) - 1;

    // Calculate subregion size and which are enabled
    let subregion_size = region_size / 8;
    let mut enabled_subregions = Vec::new();
    let mut srd_mask: u8 = 0;

    for i in 0..8u8 {
        let sr_start = aligned_base + u64::from(i) * subregion_size;
        let sr_end = sr_start + subregion_size;
        // Subregion overlaps requested range if: sr_start < end AND sr_end > start
        if sr_start < end && sr_end > start {
            enabled_subregions.push(i);
        } else {
            srd_mask |= 1 << i;
        }
    }

    Pmsav7Region {
        base: aligned_base,
        size: region_size,
        size_field,
        subregion_size,
        srd_mask,
        enabled_subregions,
    }
}

/// Check if a PMSAv7 region's enabled subregions overlap with a protected region.
fn check_pmsav7_subregion_overlap(
    region: &MemoryRegion,
    pmsav7: &Pmsav7Region,
    protected: &MemoryRegion,
) -> Option<MpuIssue> {
    for &sr in &pmsav7.enabled_subregions {
        let sr_start = pmsav7.base + u64::from(sr) * pmsav7.subregion_size;
        let sr_end = sr_start + pmsav7.subregion_size;

        // Check if this subregion overlaps with the protected region
        if sr_start < protected.end && sr_end > protected.start {
            let overlap_start = sr_start.max(protected.start);
            let overlap_end = sr_end.min(protected.end);

            return Some(MpuIssue {
                is_error: true,
                message: format!(
                    "PMSAv7 MPU subregion overlap: '{}' [{:#010x}-{:#010x}] requires MPU region \
                     [{:#010x}-{:#010x}] ({}KB), and enabled subregion {} [{:#010x}-{:#010x}] \
                     overlaps with '{}' [{:#010x}-{:#010x}] at [{:#010x}-{:#010x}]",
                    region.name,
                    region.start,
                    region.end,
                    pmsav7.base,
                    pmsav7.base + pmsav7.size,
                    pmsav7.size / 1024,
                    sr,
                    sr_start,
                    sr_end,
                    protected.name,
                    protected.start,
                    protected.end,
                    overlap_start,
                    overlap_end,
                ),
                region_name: region.name.clone(),
                suggestion: Some(suggest_aligned_address(region, protected)),
            });
        }
    }
    None
}

/// Suggest an aligned address that would avoid overlap.
fn suggest_aligned_address(region: &MemoryRegion, protected: &MemoryRegion) -> String {
    let size = region.size();
    let ideal_size = size.next_power_of_two();

    // Find the next power-of-2 aligned address after the protected region ends
    let aligned_start = (protected.end + ideal_size - 1) & !(ideal_size - 1);

    format!(
        "Move '{}' to {:#010x} (aligned to {}KB boundary) to avoid overlap with '{}'",
        region.name,
        aligned_start,
        ideal_size / 1024,
        protected.name,
    )
}

/// Calculate MPU region bloat factor.
#[allow(clippy::cast_precision_loss)] // Acceptable for ratio calculation
fn calculate_bloat_factor(region: &MemoryRegion) -> f64 {
    let pmsav7 = calculate_pmsav7_region(region.start, region.end);
    pmsav7.size as f64 / region.size() as f64
}

/// Validate memory layout for PMSAv7 compatibility.
///
/// Returns a list of issues found during validation.
#[must_use]
pub fn validate_pmsav7_layout(
    kernel_flash_start: u64,
    kernel_flash_end: u64,
    kernel_ram_start: u64,
    kernel_ram_end: u64,
    apps: &[(String, u64, u64, u64, u64)], // (name, flash_start, flash_end, ram_start, ram_end)
) -> Vec<MpuIssue> {
    let mut issues = Vec::new();

    // Build list of protected regions (kernel memory)
    let protected_regions = vec![
        MemoryRegion {
            name: "Kernel Flash".to_string(),
            start: kernel_flash_start,
            end: kernel_flash_end,
            is_kernel: true,
            is_executable: true,
        },
        MemoryRegion {
            name: "Kernel RAM".to_string(),
            start: kernel_ram_start,
            end: kernel_ram_end,
            is_kernel: true,
            is_executable: false,
        },
    ];

    // Check each app's flash region
    for (name, flash_start, flash_end, _ram_start, _ram_end) in apps {
        let app_flash = MemoryRegion {
            name: format!("App '{}' Flash", name),
            start: *flash_start,
            end: *flash_end,
            is_kernel: false,
            is_executable: true,
        };

        let pmsav7 = calculate_pmsav7_region(app_flash.start, app_flash.end);

        // Check for overlap with each protected region
        for protected in &protected_regions {
            if let Some(issue) = check_pmsav7_subregion_overlap(&app_flash, &pmsav7, protected) {
                issues.push(issue);
            }
        }

        // Check for excessive bloat (warning)
        let bloat = calculate_bloat_factor(&app_flash);
        if bloat > 4.0 {
            issues.push(MpuIssue {
                is_error: false,
                message: format!(
                    "PMSAv7 region bloat: '{}' [{:#010x}-{:#010x}] ({}KB) requires {}KB MPU region \
                     ({:.1}x bloat). Consider power-of-2 aligned placement.",
                    app_flash.name,
                    app_flash.start,
                    app_flash.end,
                    app_flash.size() / 1024,
                    pmsav7.size / 1024,
                    bloat,
                ),
                region_name: app_flash.name.clone(),
                suggestion: Some(format!(
                    "Align '{}' start address to {}KB boundary",
                    app_flash.name,
                    app_flash.size().next_power_of_two() / 1024,
                )),
            });
        }
    }

    // Check for app-to-app overlap via MPU regions (both directions)
    for (i, (name_a, flash_start_a, flash_end_a, _, _)) in apps.iter().enumerate() {
        let region_a = MemoryRegion {
            name: format!("App '{}' Flash", name_a),
            start: *flash_start_a,
            end: *flash_end_a,
            is_kernel: false,
            is_executable: true,
        };
        let pmsav7_a = calculate_pmsav7_region(region_a.start, region_a.end);

        for (name_b, flash_start_b, flash_end_b, _, _) in apps.iter().skip(i + 1) {
            let region_b = MemoryRegion {
                name: format!("App '{}' Flash", name_b),
                start: *flash_start_b,
                end: *flash_end_b,
                is_kernel: false,
                is_executable: true,
            };
            let pmsav7_b = calculate_pmsav7_region(region_b.start, region_b.end);

            // Check if pmsav7_a's enabled subregions overlap with region_b
            if let Some(issue) = check_pmsav7_subregion_overlap(&region_a, &pmsav7_a, &region_b) {
                issues.push(issue);
            }

            // Check if pmsav7_b's enabled subregions overlap with region_a
            if let Some(issue) = check_pmsav7_subregion_overlap(&region_b, &pmsav7_b, &region_a) {
                issues.push(issue);
            }
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_pmsav7_region_aligned() {
        // 128KB region starting at 128KB boundary - should be exact fit
        let region = calculate_pmsav7_region(0x20000, 0x40000);
        assert_eq!(region.base, 0x20000);
        assert_eq!(region.size, 0x20000); // 128KB
        assert_eq!(region.srd_mask, 0x00); // All subregions enabled
    }

    #[test]
    fn test_calculate_pmsav7_region_misaligned() {
        // 128KB region starting at 0x40420 - needs larger region
        let region = calculate_pmsav7_region(0x40420, 0x60420);
        assert!(region.size > 0x20000); // Must be larger than 128KB
        assert!(region.srd_mask != 0); // Some subregions disabled
    }

    #[test]
    fn test_validate_pmsav7_layout_overlap() {
        // Simulate the AST1030 problematic layout
        let issues = validate_pmsav7_layout(
            0x00000420, // kernel flash start
            0x00040420, // kernel flash end (256KB)
            0x00080420, // kernel RAM start
            0x000a0420, // kernel RAM end (128KB)
            &[
                (
                    "initiator".to_string(),
                    0x00040420,
                    0x00060420,
                    0x000a0420,
                    0x000a8420,
                ),
                (
                    "handler".to_string(),
                    0x00060420,
                    0x00080420,
                    0x000a8420,
                    0x000ac420,
                ),
            ],
        );

        // Should detect the handler flash overlapping with kernel RAM
        assert!(!issues.is_empty());
        assert!(issues.iter().any(|i| i.is_error));
    }

    #[test]
    fn test_validate_pmsav7_layout_clean() {
        // A PMSAv7-friendly layout - all power-of-2 aligned
        let issues = validate_pmsav7_layout(
            0x00000420, // kernel flash start
            0x00020000, // kernel flash end (at 128KB boundary)
            0x00060000, // kernel RAM start (at 384KB)
            0x00080000, // kernel RAM end
            &[
                (
                    "initiator".to_string(),
                    0x00020000, // 128KB aligned
                    0x00040000,
                    0x00080000,
                    0x00084000,
                ),
                (
                    "handler".to_string(),
                    0x00040000, // 256KB aligned
                    0x00060000,
                    0x00084000,
                    0x00088000,
                ),
            ],
        );

        // Should not detect any overlap errors
        let errors: Vec<_> = issues.iter().filter(|i| i.is_error).collect();
        assert!(errors.is_empty(), "Unexpected overlap errors: {:?}", errors);
    }

    #[test]
    fn test_validate_pmsav7_excessive_bloat() {
        // An 8KB app flash region that straddles a 256KB boundary (0x40000)
        // This causes massive bloat: 8KB requested -> 512KB MPU region (64x bloat)
        let issues = validate_pmsav7_layout(
            0x00000000, // kernel flash start
            0x00020000, // kernel flash end (128KB)
            0x20000000, // kernel RAM start (separate memory region)
            0x20010000, // kernel RAM end
            &[(
                "bloated_app".to_string(),
                0x0003f000, // flash start - just before 256KB boundary
                0x00041000, // flash end - just after 256KB boundary (8KB total)
                0x20010000, // RAM start
                0x20012000, // RAM end
            )],
        );

        // Should detect excessive bloat warning
        let bloat_issues: Vec<_> = issues.iter().filter(|i| !i.is_error).collect();
        assert!(
            !bloat_issues.is_empty(),
            "Expected bloat warning for 8KB region requiring 512KB MPU region"
        );
    }

    #[test]
    fn test_validate_pmsav7_no_bloat_warning_when_aligned() {
        // A well-aligned 128KB app - no bloat
        let issues = validate_pmsav7_layout(
            0x00000000, // kernel flash start
            0x00020000, // kernel flash end
            0x20000000, // kernel RAM start
            0x20010000, // kernel RAM end
            &[(
                "aligned_app".to_string(),
                0x00020000, // flash start - 128KB aligned
                0x00040000, // flash end (128KB, power-of-2)
                0x20010000,
                0x20018000,
            )],
        );

        // Should NOT detect bloat warning
        let bloat_issues: Vec<_> = issues.iter().filter(|i| !i.is_error).collect();
        assert!(
            bloat_issues.is_empty(),
            "Unexpected bloat warning for well-aligned region: {:?}",
            bloat_issues
        );
    }

    #[test]
    fn test_validate_pmsav7_app_to_app_overlap() {
        // App order should NOT matter - the validation checks both directions.
        //
        // App A (victim): 32KB at 0x30000-0x38000
        // App B (bloated): 8KB at 0x3F000-0x41000, straddles 256KB boundary
        //   -> PMSAv7 region: base=0x0, size=512KB, subregion size=64KB
        //   -> Enabled subregions include SR3 (0x30000-0x40000) which overlaps App A
        //
        // Result: App B's SR3 overlaps with App A's actual memory -> MPU003
        let issues = validate_pmsav7_layout(
            0x00000000, // kernel flash start
            0x00010000, // kernel flash end (64KB)
            0x20000000, // kernel RAM start
            0x20010000, // kernel RAM end
            &[
                (
                    "victim_app".to_string(),
                    0x00030000, // flash start
                    0x00038000, // flash end (32KB)
                    0x20010000,
                    0x20018000,
                ),
                (
                    "bloated_app".to_string(),
                    0x0003f000, // flash start - straddles 256KB boundary
                    0x00041000, // flash end (8KB, but bloats to 512KB MPU region)
                    0x20018000,
                    0x20020000,
                ),
            ],
        );

        // Should detect app-to-app overlap regardless of app order
        let overlap_issues: Vec<_> = issues.iter().filter(|i| i.is_error).collect();
        assert!(
            !overlap_issues.is_empty(),
            "Expected app-to-app overlap when bloated_app's SR3 overlaps victim_app"
        );
    }

    #[test]
    fn test_validate_pmsav7_no_app_overlap_when_separated() {
        // Two well-separated, aligned apps - no overlap
        let issues = validate_pmsav7_layout(
            0x00000000, // kernel flash start
            0x00010000, // kernel flash end
            0x20000000, // kernel RAM start
            0x20010000, // kernel RAM end
            &[
                (
                    "app_a".to_string(),
                    0x00010000, // 64KB aligned
                    0x00020000,
                    0x20010000,
                    0x20018000,
                ),
                (
                    "app_b".to_string(),
                    0x00020000, // 128KB aligned, after app_a
                    0x00030000,
                    0x20018000,
                    0x20020000,
                ),
            ],
        );

        // Should NOT detect any overlap errors
        let overlap_issues: Vec<_> = issues.iter().filter(|i| i.is_error).collect();
        assert!(
            overlap_issues.is_empty(),
            "Unexpected overlap errors for well-separated apps: {:?}",
            overlap_issues
        );
    }
}
