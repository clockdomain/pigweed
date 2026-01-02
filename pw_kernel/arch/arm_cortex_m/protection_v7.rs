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

use kernel_config::{CortexMKernelConfigInterface as _, KernelConfig};
use memory_config::{MemoryRegion, MemoryRegionType};

use crate::regs::Regs;
use crate::regs::mpu::*;

/// PMSAv7 MPU Region
#[derive(Copy, Clone)]
pub struct MpuRegion {
    #[allow(dead_code)]
    pub rbar: RbarVal,
    #[allow(dead_code)]
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
                /* tex */ 0b001,  // Normal memory, outer and inner write-back
                /* s */ true, /* c */ true, /* b */ true,
                RasrAp::RoAny,
            ),
            MemoryRegionType::ReadWriteData => (
                /* xn */ true,
                /* tex */ 0b001,  // Normal memory, outer and inner write-back
                /* s */ false, /* c */ true, /* b */ true,
                RasrAp::RwAny,
            ),
            MemoryRegionType::ReadOnlyExecutable => (
                /* xn */ false,
                /* tex */ 0b001,  // Normal memory, outer and inner write-back
                /* s */ true, /* c */ true, /* b */ true,
                RasrAp::RoAny,
            ),
            MemoryRegionType::ReadWriteExecutable => (
                /* xn */ false,
                /* tex */ 0b001,  // Normal memory, outer and inner write-back
                /* s */ true, /* c */ true, /* b */ true,
                RasrAp::RwAny,
            ),
            MemoryRegionType::Device => (
                /* xn */ true,
                /* tex */ 0b000,  // Device memory
                /* s */ true, /* c */ false, /* b */ true,
                RasrAp::RoAny,
            ),
        };

        #[expect(clippy::cast_possible_truncation)]
        Self {
            rbar: RbarVal::const_default()
                .with_valid(false)  // Region selected by RNR, not by RBAR.REGION
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
            4  // Minimum 32 bytes
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
        
        if requested_size >= MAX_REGION_SIZE {
            // Use maximum region size with no sub-regions disabled
            return AlignedRegion {
                base: 0,
                size_field: 30, // 2GB = 2^31, SIZE = 31-1 = 30
                srd_mask: 0,
            };
        }
        
        // Find the smallest power-of-2 region that can cover the requested range
        // Start with the requested size, round up to next power of 2
        let mut region_size = 32; // Minimum 32 bytes
        while region_size < requested_size {
            region_size *= 2;
            if region_size > MAX_REGION_SIZE {
                // Fall back to max size
                return AlignedRegion {
                    base: 0,
                    size_field: 30,
                    srd_mask: 0,
                };
            }
        }
        
        // Find an aligned base that covers the requested range
        // The base must be aligned to the region size
        // 
        // CRITICAL: We must not align down across major memory boundaries.
        // On AST1030: Flash ends at 0x3FFFF, RAM starts at 0x40000.
        // If we blindly align down, a region starting at 0x60420 (in RAM) could
        // align to 0x40000 or even 0x00000, causing it to overlap with flash.
        //
        // Strategy: Try aligning down first, but if that crosses the start of
        // the current 256KB page (which typically separates flash/RAM), align
        // to the page boundary instead. This prevents cross-boundary issues while
        // still allowing efficient region packing within the same memory type.
        const PAGE_256KB: usize = 0x40000;
        let start_page = start & !(PAGE_256KB - 1);
        
        let naive_aligned_base = start & !(region_size - 1); // Align down to region_size
        
        // If alignment crosses below the start's 256KB page boundary, use the page boundary instead
        let aligned_base = if naive_aligned_base < start_page {
            start_page
        } else {
            naive_aligned_base
        };
        
        // Debug logging to trace alignment decisions
        // This is const fn so we can't use pw_log, but the values will be visible in MPU dumps
        
        // Check if this aligned region covers the end address
        // If not, we need a larger region
        let mut final_base = aligned_base;
        let mut final_size = region_size;
        
        while final_base + final_size < end {
            final_size *= 2;
            let candidate_base = start & !(final_size - 1);
            
            // Apply the same page boundary constraint
            final_base = if candidate_base < start_page {
                start_page
            } else {
                candidate_base
            };
            
            if final_size > MAX_REGION_SIZE {
                // Fall back to max size at base 0
                return AlignedRegion {
                    base: 0,
                    size_field: 30,
                    srd_mask: 0,
                };
            }
        }
        
        // Calculate SIZE field: log2(region_size) - 1
        let size_field = Self::calculate_size_field(final_size);
        
        // Calculate sub-region disable mask
        // Each sub-region is region_size / 8
        let subregion_size = final_size / 8;
        let mut srd_mask: u8 = 0;
        
        // Disable sub-regions that fall outside [start, end)
        let mut i = 0;
        while i < 8 {
            let subregion_start = final_base + i * subregion_size;
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
            base: final_base,
            size_field,
            srd_mask,
        }
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
        
        // NOTE: We do NOT disable the MPU during reconfiguration on ARMv7-M.
        // Disabling the MPU removes all memory protections, which can cause:
        // - Speculative memory accesses corrupting the stack
        // - Exception frames being overwritten
        // - Unpredictable behavior during context switches
        //
        // PMSAv7 supports updating regions while enabled - just ensure barriers after.
        // Configure HFNMIENA=false (faults during NMI/HardFault) and PRIVDEFENA=true
        // (privileged code can access unmapped regions).
        mpu.ctrl.write(
            mpu.ctrl
                .read()
                .with_enable(true)  // Keep MPU enabled during update
                .with_hfnmiena(false)
                .with_privdefena(true),
        );

        // Write MPU regions inline (avoiding function call overhead that can corrupt registers/stack)
        for (index, region) in self.mpu_regions.iter().enumerate() {
            pw_assert::debug_assert!(index < 255);
            #[expect(clippy::cast_possible_truncation)]
            {
                mpu.rnr.write(RnrVal::default().with_region(index as u8));
            }
            mpu.rbar.write(region.rbar);
            mpu.rasr.write(region.rasr);
        }
        
        // Enable the MPU
        mpu.ctrl.write(mpu.ctrl.read().with_enable(true));
        
        // CRITICAL: ARMv7-M requires explicit memory barriers after MPU configuration changes.
        // Per ARM DDI 0403E.e Section B3.5.8:
        // - DSB ensures all MPU register writes are complete before proceeding
        // - ISB flushes the instruction pipeline, ensuring subsequent instructions
        //   are fetched and executed with the new MPU configuration active
        //
        // Without these barriers, the processor may execute cached instructions or
        // access memory using stale MPU settings, causing spurious faults and
        // infinite context switch loops.
        unsafe {
            core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
        }
    }

}

// Removed dump() method - debug logging not needed

/// Initialize the MPU for supporting user space memory protection (PMSAv7).
/// 
/// PMSAv7 doesn't use MAIR registers - memory attributes are encoded directly
/// in the RASR register using TEX, C, B, S fields.
pub fn init() {
    // PMSAv7 doesn't require any initialization beyond what's done in write().
    // Memory attributes are inline in RASR, unlike PMSAv8's MAIR.
}

impl memory_config::MemoryConfig for MemoryConfig {
    const KERNEL_THREAD_MEMORY_CONFIG: Self = Self::const_new(&[MemoryRegion::new(
        MemoryRegionType::ReadWriteExecutable,
        0x0000_0000,
        0xffff_ffff,
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
