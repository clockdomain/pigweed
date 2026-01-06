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

//! PMSAv7 (ARMv7-M) MPU implementation
//!
//! PMSAv7 (Protected Memory System Architecture v7) is the MPU architecture
//! used in ARMv7-M processors (Cortex-M3, Cortex-M4, Cortex-M7). Unlike PMSAv8,
//! PMSAv7 requires:
//!
//! - **Power-of-2 region sizes**: Regions must be 32 bytes to 4GB, always a power of 2
//! - **Size-aligned bases**: Region base must be aligned to its size
//! - **Sub-region disable (SRD)**: Each region can disable up to 8 sub-regions
//!   (each 1/8th of the total region) to handle non-power-of-2 ranges
//! - **Inline memory attributes**: TEX, C, B, S fields in RASR (no MAIR registers)
//!
//! This implementation uses sub-regions to map arbitrary memory ranges to
//! PMSAv7's power-of-2 constraints.

use kernel_config::{CortexMKernelConfigInterface as _, KernelConfig};
use memory_config::{MemoryRegion, MemoryRegionType};

use crate::regs::Regs;
use crate::regs::mpu::*;

/// PMSAv7 MPU Region
#[derive(Copy, Clone)]
pub struct MpuRegion {
    pub rbar: RbarVal,
    pub rasr: RasrVal,
}

/// Helper structure for PMSAv7 aligned region calculation
struct AlignedRegion {
    base: usize,
    size_field: u8,
    srd_mask: u8,
}

impl MpuRegion {
    pub const fn const_default() -> Self {
        Self {
            rbar: RbarVal::const_default(),
            rasr: RasrVal::const_default(),
        }
    }

    pub const fn from_memory_region(region: &MemoryRegion) -> Self {
        // PMSAv7 requires power-of-2 sized regions aligned to their size.
        // Use sub-regions to handle arbitrary ranges.
        let aligned_region = Self::calculate_aligned_region(region.start, region.end);

        let (xn, tex, s, c, b, ap) = match region.ty {
            MemoryRegionType::ReadOnlyData => (
                /* xn */ true,
                /* tex */ 0b001, // Normal memory, outer and inner write-back
                /* s */ true,
                /* c */ true,
                /* b */ true,
                RasrAp::RoAny,
            ),
            MemoryRegionType::ReadWriteData => (
                /* xn */ true,
                /* tex */ 0b001, // Normal memory, outer and inner write-back
                /* s */ false,
                /* c */ true,
                /* b */ true,
                RasrAp::RwAny,
            ),
            MemoryRegionType::ReadOnlyExecutable => (
                /* xn */ false,
                /* tex */ 0b001, // Normal memory, outer and inner write-back
                /* s */ true,
                /* c */ true,
                /* b */ true,
                RasrAp::RoAny,
            ),
            MemoryRegionType::ReadWriteExecutable => (
                /* xn */ false,
                /* tex */ 0b001, // Normal memory, outer and inner write-back
                /* s */ true,
                /* c */ true,
                /* b */ true,
                RasrAp::RwAny,
            ),
            MemoryRegionType::Device => (
                /* xn */ true,
                /* tex */ 0b000, // Device memory
                /* s */ true,
                /* c */ false,
                /* b */ true,
                RasrAp::RoAny,
            ),
        };

        #[expect(clippy::cast_possible_truncation)]
        Self {
            rbar: RbarVal::const_default()
                .with_valid(false) // Region selected by RNR, not by RBAR.REGION
                .with_addr(aligned_region.base as u32),

            rasr: RasrVal::const_default()
                .with_enable(true)
                .with_size(aligned_region.size_field)
                .with_srd(aligned_region.srd_mask)
                .with_tex(tex)
                .with_s(s)
                .with_c(c)
                .with_b(b)
                .with_ap(ap)
                .with_xn(xn),
        }
    }

    /// Helper to calculate SIZE field from region size in bytes
    const fn calculate_size_field(size_bytes: usize) -> u8 {
        // SIZE = log2(size) - 1
        // Find the position of the highest set bit
        let mut size = size_bytes;
        let mut bits = 0;
        while size > 1 {
            size >>= 1;
            bits += 1;
        }
        // SIZE field is bits - 1, minimum is 4 (32 bytes)
        if bits < 5 {
            4 // Minimum 32 bytes
        } else {
            #[expect(clippy::cast_possible_truncation)]
            ((bits - 1) as u8)
        }
    }

    /// Calculate an aligned region that covers [start, end) using sub-regions
    const fn calculate_aligned_region(start: usize, end: usize) -> AlignedRegion {
        let requested_size = end - start;

        // PMSAv7 maximum region size is 4GB (2^32), but SIZE field max is 31 (2^32)
        // For very large regions (like kernel's full address space), use maximum size
        const MAX_REGION_SIZE: usize = 0x8000_0000; // 2GB, SIZE=30

        if requested_size > MAX_REGION_SIZE {
            panic!("Requested memory region size exceeds PMSAv7 limits");
        }

        // Find the smallest power-of-2 region that can cover the requested range
        // Start with the requested size, round up to next power of 2
        let mut region_size = 32; // Minimum 32 bytes
        while region_size < requested_size {
            region_size *= 2;
            if region_size > MAX_REGION_SIZE {
                panic!("Requested memory region requires alignment/size exceeding PMSAv7 limits");
            }
        }

        // Find an aligned base that covers the requested range
        // The base must be aligned to the region size
        let mut aligned_base = start & !(region_size - 1); // Align down to region_size

        // Check if this aligned region covers the end address
        // If not, we need a larger region
        while aligned_base + region_size < end {
            region_size *= 2;
            aligned_base = start & !(region_size - 1);

            if region_size > MAX_REGION_SIZE {
                panic!("Requested memory region requires alignment/size exceeding PMSAv7 limits");
            }
        }

        // Calculate SIZE field: log2(region_size) - 1
        let size_field = Self::calculate_size_field(region_size);

        // Calculate sub-region disable mask
        // Each sub-region is region_size / 8
        let subregion_size = region_size / 8;
        let mut srd_mask: u8 = 0;

        // SECURITY WARNING: Sub-region over-provisioning
        // ===============================================
        // This implementation has a known security trade-off: it grants access to entire
        // sub-regions if they have ANY overlap with the requested range. This means up to
        // (region_size / 8) - 1 bytes can be accessible beyond the requested boundaries.
        //
        // EXAMPLE:
        //   Requested range: [0x1000, 0x1100) - 256 bytes
        //   Aligned region:  [0x1000, 0x1800) - 2KB (power-of-2 requirement)
        //   Sub-region size: 256 bytes (2KB / 8)
        //   Sub-region 1:    [0x1100, 0x1300) - starts at requested end
        //   Result: Sub-region 1 is FULLY enabled, exposing [0x1100, 0x1300)
        //           This grants 512 bytes of unintended access beyond 0x1100
        //
        // ROOT CAUSE - PMSAv7 Hardware Constraints:
        //   1. Regions must be power-of-2 sized (32B to 4GB)
        //   2. Region base must be aligned to region size
        //   3. Each region has exactly 8 sub-regions (all equal size)
        //   4. Sub-regions can only be fully enabled or fully disabled (no partial)
        //   5. Only 8 MPU regions available system-wide
        //
        // WHY NOT FIXED:
        //   Precise coverage requires splitting into multiple MPU regions, but:
        //   - Would consume more of the limited 8 MPU regions
        //   - Complex algorithm to optimally split arbitrary ranges
        //   - May not always be possible (e.g., 9 memory regions in system)
        //   - Current approach guarantees coverage with simple logic
        //
        // SECURITY IMPLICATIONS:
        //   - Low to Medium severity depending on memory layout
        //   - Could expose heap metadata, adjacent data structures, or other process memory
        //   - Violates principle of least privilege
        //   - Particularly concerning at userspace/kernel boundaries
        //
        // MITIGATION:
        //   - Design memory layout with sub-region boundaries in mind
        //   - Place guard regions between sensitive structures
        //   - Align allocations to sub-region boundaries when possible
        //   - Consider PMSAv8 architectures (ARMv8-M) which don't have this limitation
        //
        // This is an ACCEPTED RISK in the current implementation prioritizing simplicity
        // and guaranteed coverage over precision.
        //
        // Disable sub-regions that fall outside [start, end)
        let mut i = 0;
        while i < 8 {
            let subregion_start = aligned_base + i * subregion_size;
            let subregion_end = subregion_start + subregion_size;

            // Disable if this sub-region doesn't overlap with [start, end)
            // A sub-region overlaps if: subregion_start < end AND subregion_end > start
            let overlaps = subregion_start < end && subregion_end > start;
            if !overlaps {
                srd_mask |= 1 << i;
            }
            i += 1;
        }

        AlignedRegion {
            base: aligned_base,
            size_field,
            srd_mask,
        }
    }

    pub fn write(&self, mpu: &mut crate::regs::mpu::Mpu, region_number: usize) {
        pw_log::debug!(
            "MPU[{}]: RBAR=0x{:08X} RASR=0x{:08X}",
            region_number as usize,
            self.rbar.0 as usize,
            self.rasr.0 as usize
        );

        pw_assert::debug_assert!(region_number < 255);
        #[expect(clippy::cast_possible_truncation)]
        {
            mpu.rnr
                .write(RnrVal::default().with_region(region_number as u8));
        }
        mpu.rbar.write(self.rbar);
        mpu.rasr.write(self.rasr);
    }
}

/// Represents the full configuration of the Cortex-M memory configuration
/// through the MPU block for ARMv7-M processors (PMSAv7).
pub struct MemoryConfig {
    mpu_regions: [MpuRegion; KernelConfig::NUM_MPU_REGIONS],
    generic_regions: &'static [MemoryRegion],
}

impl MemoryConfig {
    /// Create a new `MemoryConfig` in a `const` context
    ///
    /// # Panics
    /// Will panic if the current target's MPU does not support enough regions
    /// to represent `regions`.
    #[must_use]
    pub const fn const_new(regions: &'static [MemoryRegion]) -> Self {
        let mut mpu_regions = [MpuRegion::const_default(); KernelConfig::NUM_MPU_REGIONS];
        let mut i = 0;
        while i < regions.len() {
            mpu_regions[i] = MpuRegion::from_memory_region(&regions[i]);
            i += 1;
        }
        Self {
            mpu_regions,
            generic_regions: regions,
        }
    }

    /// Write this memory configuration to the MPU registers.
    ///
    /// # Safety
    /// Caller must ensure that it is safe and sound to update the MPU with this
    /// memory config.
    pub unsafe fn write(&self) {
        let mut mpu = Regs::get().mpu;

        // Disable MPU before configuration
        mpu.ctrl.write(
            mpu.ctrl
                .read()
                .with_enable(false)
                .with_hfnmiena(false)
                .with_privdefena(true),
        );

        pw_log::info!(
            "Programming {} MPU regions (PMSAv7)",
            self.mpu_regions.len() as usize
        );

        for (index, region) in self.mpu_regions.iter().enumerate() {
            region.write(&mut mpu, index);
        }

        // Enable the MPU
        mpu.ctrl.write(mpu.ctrl.read().with_enable(true));
    }

    /// Log the details of the memory configuration.
    pub fn dump(&self) {
        for (index, region) in self.mpu_regions.iter().enumerate() {
            pw_log::debug!(
                "MPU region {}: RBAR={:#010x}, RASR={:#010x}",
                index as usize,
                region.rbar.0 as usize,
                region.rasr.0 as usize
            );
        }
    }
}

/// Initialize the MPU for supporting user space memory protection (PMSAv7).
///
/// PMSAv7 doesn't use MAIR registers - memory attributes are encoded directly
/// in the RASR register using TEX, C, B, S fields.
pub fn init() {
    // PMSAv7 doesn't require any initialization beyond what's done in write().
    // Memory attributes are inline in RASR, unlike PMSAv8's MAIR.
}

impl memory_config::MemoryConfig for MemoryConfig {
    // We limit the kernel region to 2GB (0x8000_0000) to satisfy the PMSAv7 implementation's
    // MAX_REGION_SIZE constraint. This covers the typical Flash/RAM/Peripheral range
    // for Cortex-M devices.
    const KERNEL_THREAD_MEMORY_CONFIG: Self = Self::const_new(&[MemoryRegion::new(
        MemoryRegionType::ReadWriteExecutable,
        0x0000_0000,
        0x8000_0000,
    )]);

    fn range_has_access(
        &self,
        access_type: MemoryRegionType,
        start_addr: usize,
        end_addr: usize,
    ) -> bool {
        let validation_region = MemoryRegion::new(access_type, start_addr, end_addr);
        MemoryRegion::regions_have_access(self.generic_regions, &validation_region)
    }
}
