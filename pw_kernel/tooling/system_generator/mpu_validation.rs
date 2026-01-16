// Copyright 2025 The Pigweed Authors
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

//! MPU-aware memory layout validation for the system generator.
//!
//! This module validates that memory layouts are compatible with hardware
//! Memory Protection Unit (MPU) constraints, particularly for:
//! - ARMv7-M PMSAv7 (power-of-2 regions with subregion disable)
//! - ARMv8-M PMSAv8 (arbitrary regions with 32-byte granularity)
//!
//! The key insight is that PMSAv7's power-of-2 alignment requirements can cause
//! MPU regions to "bloat" beyond their requested size, potentially overlapping
//! with kernel memory and causing hard-to-diagnose faults at runtime.

use std::fmt;

use serde::{Deserialize, Serialize};

/// MPU validation severity level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MpuValidationMode {
    /// Fail the build if any MPU compatibility issues are detected.
    Strict,
    /// Emit warnings for MPU compatibility issues but continue.
    #[default]
    Warn,
    /// Only emit informational messages.
    Permissive,
}

/// Severity of an MPU validation issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

impl fmt::Display for IssueSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IssueSeverity::Error => write!(f, "error"),
            IssueSeverity::Warning => write!(f, "warning"),
            IssueSeverity::Info => write!(f, "info"),
        }
    }
}

/// A detected MPU compatibility issue.
#[derive(Clone, Debug)]
pub struct MpuIssue {
    /// Error code (e.g., "MPU001")
    pub code: &'static str,
    /// Human-readable description
    pub message: String,
    /// Name of the affected region
    pub region_name: String,
    /// Suggested fix, if available
    pub suggestion: Option<String>,
}

impl fmt::Display for MpuIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]: {}", self.code, self.message)?;
        if let Some(suggestion) = &self.suggestion {
            write!(f, "\n  suggestion: {}", suggestion)?;
        }
        Ok(())
    }
}

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

/// A memory region for validation purposes.
#[derive(Clone, Debug)]
pub struct MemoryRegion {
    pub name: String,
    pub start: u64,
    pub end: u64,
    pub is_kernel: bool,
    pub is_executable: bool,
}

impl MemoryRegion {
    pub fn size(&self) -> u64 {
        self.end - self.start
    }
}

/// Calculate the PMSAv7 aligned region for a memory range.
///
/// PMSAv7 requires:
/// - Power-of-2 region sizes (32 bytes to 4GB)
/// - Region base aligned to region size
/// - 8 subregions per region (each 1/8 of total size)
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
        let sr_start = aligned_base + (i as u64) * subregion_size;
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
        let sr_start = pmsav7.base + (sr as u64) * pmsav7.subregion_size;
        let sr_end = sr_start + pmsav7.subregion_size;

        // Check if this subregion overlaps with the protected region
        if sr_start < protected.end && sr_end > protected.start {
            let overlap_start = sr_start.max(protected.start);
            let overlap_end = sr_end.min(protected.end);

            return Some(MpuIssue {
                code: "MPU001",
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
fn calculate_bloat_factor(region: &MemoryRegion) -> f64 {
    let pmsav7 = calculate_pmsav7_region(region.start, region.end);
    pmsav7.size as f64 / region.size() as f64
}

/// Validate memory layout for PMSAv7 compatibility.
///
/// Returns a list of issues found during validation.
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
                code: "MPU002",
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

    // Check for app-to-app overlap via MPU regions
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

            // Check if pmsav7_a's enabled subregions overlap with region_b
            if let Some(mut issue) = check_pmsav7_subregion_overlap(&region_a, &pmsav7_a, &region_b)
            {
                issue.code = "MPU003";
                issues.push(issue);
            }
        }
    }

    issues
}

/// Format a detailed PMSAv7 analysis for a memory region.
pub fn format_pmsav7_analysis(name: &str, start: u64, end: u64) -> String {
    let pmsav7 = calculate_pmsav7_region(start, end);
    let size = end - start;

    let mut output = format!(
        "{} [{:#010x}-{:#010x}] ({}KB):\n",
        name,
        start,
        end,
        size / 1024
    );
    output.push_str(&format!(
        "  PMSAv7 region: base={:#010x}, size={}KB, SIZE_FIELD={}\n",
        pmsav7.base,
        pmsav7.size / 1024,
        pmsav7.size_field
    ));
    output.push_str(&format!(
        "  Subregion size: {}KB\n",
        pmsav7.subregion_size / 1024
    ));
    output.push_str(&format!(
        "  SRD mask: {:#04x} = {:#010b}\n",
        pmsav7.srd_mask, pmsav7.srd_mask
    ));
    output.push_str(&format!(
        "  Enabled subregions: {:?}\n",
        pmsav7.enabled_subregions
    ));

    for sr in 0..8u8 {
        let sr_start = pmsav7.base + (sr as u64) * pmsav7.subregion_size;
        let sr_end = sr_start + pmsav7.subregion_size;
        let status = if pmsav7.enabled_subregions.contains(&sr) {
            "ENABLED"
        } else {
            "disabled"
        };
        output.push_str(&format!(
            "    SR{}: {:#010x}-{:#010x} [{}]\n",
            sr, sr_start, sr_end, status
        ));
    }

    output
}

/// Generate a suggested PMSAv7-friendly memory layout.
pub fn suggest_pmsav7_friendly_layout(
    vector_table_size: u64,
    kernel_flash_size: u64,
    kernel_ram_size: u64,
    apps: &[(String, u64, u64)], // (name, flash_size, ram_size)
) -> String {
    let mut output = String::new();
    output.push_str("Suggested PMSAv7-friendly memory layout:\n\n");

    // Calculate optimal kernel flash size that ends at power-of-2 boundary
    let max_app_flash = apps.iter().map(|(_, fs, _)| *fs).max().unwrap_or(0);
    let app_flash_alignment = max_app_flash.next_power_of_two();

    // Kernel flash ends at alignment boundary
    let kernel_flash_end = ((vector_table_size + kernel_flash_size + app_flash_alignment - 1)
        / app_flash_alignment)
        * app_flash_alignment;
    let adjusted_kernel_flash_size = kernel_flash_end - vector_table_size;

    output.push_str(&format!(
        "  Vector table:  {:#010x} - {:#010x} ({}KB)\n",
        0,
        vector_table_size,
        vector_table_size / 1024
    ));
    output.push_str(&format!(
        "  Kernel flash:  {:#010x} - {:#010x} ({}KB, ends at power-of-2 boundary)\n",
        vector_table_size,
        kernel_flash_end,
        adjusted_kernel_flash_size / 1024
    ));

    // Place app flash regions
    let mut next_flash = kernel_flash_end;
    for (name, flash_size, _) in apps {
        let aligned_size = flash_size.next_power_of_two();
        let aligned_start = (next_flash + aligned_size - 1) & !(aligned_size - 1);
        output.push_str(&format!(
            "  App '{}' flash: {:#010x} - {:#010x} ({}KB, power-of-2 aligned)\n",
            name,
            aligned_start,
            aligned_start + aligned_size,
            aligned_size / 1024
        ));
        next_flash = aligned_start + aligned_size;
    }

    // Kernel RAM after all flash
    let kernel_ram_aligned = kernel_ram_size.next_power_of_two();
    let kernel_ram_start = (next_flash + kernel_ram_aligned - 1) & !(kernel_ram_aligned - 1);
    output.push_str(&format!(
        "  Kernel RAM:    {:#010x} - {:#010x} ({}KB)\n",
        kernel_ram_start,
        kernel_ram_start + kernel_ram_aligned,
        kernel_ram_aligned / 1024
    ));

    // App RAM after kernel RAM
    let mut next_ram = kernel_ram_start + kernel_ram_aligned;
    for (name, _, ram_size) in apps {
        output.push_str(&format!(
            "  App '{}' RAM:   {:#010x} - {:#010x} ({}KB)\n",
            name,
            next_ram,
            next_ram + ram_size,
            ram_size / 1024
        ));
        next_ram += ram_size;
    }

    output.push_str(&format!(
        "\n  Total memory used: {:#010x} ({}KB)\n",
        next_ram,
        next_ram / 1024
    ));

    output
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
            0x000A0420, // kernel RAM end (128KB)
            &[
                (
                    "initiator".to_string(),
                    0x00040420,
                    0x00060420,
                    0x000A0420,
                    0x000A8420,
                ),
                (
                    "handler".to_string(),
                    0x00060420,
                    0x00080420,
                    0x000A8420,
                    0x000AC420,
                ),
            ],
        );

        // Should detect the handler flash overlapping with kernel RAM
        assert!(!issues.is_empty());
        assert!(issues.iter().any(|i| i.code == "MPU001"));
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
        let errors: Vec<_> = issues.iter().filter(|i| i.code == "MPU001").collect();
        assert!(errors.is_empty(), "Unexpected overlap errors: {:?}", errors);
    }
}
